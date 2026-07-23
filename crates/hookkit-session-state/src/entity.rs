use crate::{
    JournalEntryId, Result, StateError, atomic_replace, atomic_write, create_private_dir_all,
    sha256_bytes, sync_directory, unique_id, validate_identifier,
};
use fs2::FileExt;
use hookkit_core::Utf8PathBuf;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeSet, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Validated name and nonzero schema version for a journal entity.
pub struct EntityId {
    name: String,
    version: u32,
}

impl EntityId {
    /// Creates an entity identity.
    ///
    /// `name` must be a safe state identifier and `version` must be nonzero.
    pub fn new(name: impl Into<String>, version: u32) -> Result<Self> {
        let name = name.into();
        validate_identifier(&name)?;
        if version == 0 {
            return Err(StateError::InvalidIdentifier(
                "entity version 0".to_string(),
            ));
        }
        Ok(Self { name, version })
    }

    /// Returns the validated entity name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the nonzero entity schema version.
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Persistence semantics for an entity's aggregate projection.
pub enum EntityMode {
    /// A work window. Acknowledgement removes the aggregate contribution of
    /// the covered generations.
    Windowed,
    /// Knowledge accumulated for the life of the entity. Compaction moves
    /// covered events into a durable checkpoint.
    Monotonic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Automatic compaction threshold for a monotonic entity.
pub enum CompactionPolicy {
    /// Compact only when an operation explicitly requests it.
    #[default]
    Manual,
    /// Compact when a snapshot covers at least this many pending entries.
    AfterEntries(usize),
    /// Compact when pending generation files occupy at least this many bytes.
    AfterBytes(u64),
}

/// Projection contract for a typed aggregate derived from journal events.
pub trait JournalEntity: Serialize + DeserializeOwned + Clone {
    /// Append-only event from which the aggregate is projected.
    type Event: Serialize + DeserializeOwned + Clone;

    /// Creates the projection state used when no checkpoint exists.
    fn empty() -> Self;
    /// Applies one journal event to the projection deterministically.
    fn apply(&mut self, event: &Self::Event);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// One content-addressed event read from an entity generation.
pub struct EntityRecord<T> {
    id: String,
    event_key: String,
    event: T,
}

impl<T> EntityRecord<T> {
    /// Returns the SHA-256-derived record identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the application-supplied idempotency/provenance key.
    pub fn event_key(&self) -> &str {
        &self.event_key
    }

    /// Returns the typed event payload.
    pub fn event(&self) -> &T {
        &self.event
    }
}

#[derive(Debug)]
/// Consistent aggregate and pending-event snapshot passed to an operation.
///
/// The view is valid only for the duration of the operation. Its projection
/// includes the durable checkpoint and every covered pending generation.
pub struct EntityView<'a, E: JournalEntity> {
    state: &'a E,
    events: &'a [EntityRecord<E::Event>],
    new_event_offset: usize,
    generations: &'a [String],
}

impl<'a, E: JournalEntity> EntityView<'a, E> {
    /// Returns the fully projected aggregate state.
    pub fn state(&self) -> &E {
        self.state
    }

    /// Returns all event records from covered pending generations.
    ///
    /// Events already represented by a projection cache appear first; use
    /// [`Self::new_events`] to inspect only newly applied records.
    pub fn events(&self) -> &[EntityRecord<E::Event>] {
        self.events
    }

    /// Events that were not already represented by the projection cache.
    pub fn new_events(&self) -> &[EntityRecord<E::Event>] {
        &self.events[self.new_event_offset..]
    }

    /// Returns identifiers of the pending generations covered by this view.
    pub fn generations(&self) -> &[String] {
        self.generations
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Persistence action applied after an entity operation succeeds.
pub enum EntityDisposition {
    /// Keep pending generations and cache the current projection.
    Retain,
    /// Remove covered generations without checkpointing them.
    ///
    /// This is valid only for [`EntityMode::Windowed`].
    Acknowledge,
    /// Checkpoint the current projection, then remove covered generations.
    Compact,
}

#[derive(Debug)]
/// Value returned by an operation together with its requested disposition.
pub struct EntityOutcome<T> {
    value: T,
    disposition: EntityDisposition,
}

#[derive(Debug, thiserror::Error)]
/// Distinguishes storage failures from a client operation's domain error.
pub enum EntityOperationError<E> {
    /// Session-state storage or serialization failure.
    #[error(transparent)]
    State(#[from] StateError),
    /// Error returned by the client operation.
    #[error("entity operation failed: {0}")]
    Operation(E),
}

impl<T> EntityOutcome<T> {
    /// Returns `value` while retaining covered pending generations.
    pub fn retain(value: T) -> Self {
        Self {
            value,
            disposition: EntityDisposition::Retain,
        }
    }

    /// Returns `value` and acknowledges covered windowed generations.
    pub fn acknowledge(value: T) -> Self {
        Self {
            value,
            disposition: EntityDisposition::Acknowledge,
        }
    }

    /// Returns `value` and checkpoints the projected aggregate.
    pub fn compact(value: T) -> Self {
        Self {
            value,
            disposition: EntityDisposition::Compact,
        }
    }

    /// Returns the requested post-operation disposition.
    pub fn disposition(&self) -> EntityDisposition {
        self.disposition
    }

    /// Consumes the outcome and returns its operation value.
    pub fn into_value(self) -> T {
        self.value
    }
}

#[derive(Debug, Clone)]
/// Concurrent append-only NDJSON journal with a typed aggregate projection.
///
/// Appenders coordinate only around the active generation. Consumers rotate
/// that generation, hold an exclusive consumer lock, project a consistent
/// snapshot, and atomically retain, acknowledge, or compact it.
pub struct EntityJournal<E: JournalEntity> {
    directory: PathBuf,
    mode: EntityMode,
    compaction_policy: CompactionPolicy,
    marker: PhantomData<E>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Descriptor {
    schema_version: u32,
    mode: EntityMode,
    representation: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveGeneration {
    id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Checkpoint<E> {
    revision: u64,
    state: E,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionCache<E> {
    checkpoint_revision: u64,
    generations: Vec<String>,
    state: E,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TransitionKind {
    Acknowledge,
    Compact,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Transition<E> {
    kind: TransitionKind,
    generations: Vec<String>,
    checkpoint: Option<Checkpoint<E>>,
}

struct GenerationBatch<T> {
    _consumer_lock: FileLock,
    generations: Vec<String>,
    records: Vec<GeneratedRecord<T>>,
    bytes: u64,
}

impl<E: JournalEntity> EntityJournal<E> {
    pub(crate) fn open(root: PathBuf, id: EntityId, mode: EntityMode) -> Result<Self> {
        let directory = root.join(id.name()).join(format!("v{}", id.version()));
        create_private_dir_all(&directory.join("generations"))?;
        let descriptor_path = directory.join("descriptor.json");
        let desired = Descriptor {
            schema_version: 1,
            mode,
            representation: "ndjson-generations".to_string(),
        };
        if descriptor_path.exists() {
            let actual: Descriptor = serde_json::from_slice(&std::fs::read(&descriptor_path)?)?;
            if actual.mode != mode || actual.representation != desired.representation {
                return Err(StateError::EntityConfiguration(format!(
                    "entity `{}` v{} was opened with incompatible mode or representation",
                    id.name(),
                    id.version()
                )));
            }
        } else {
            atomic_write(&descriptor_path, &serde_json::to_vec_pretty(&desired)?)?;
        }
        Ok(Self {
            directory,
            mode,
            compaction_policy: CompactionPolicy::Manual,
            marker: PhantomData,
        })
    }

    /// Returns the versioned on-disk entity directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the entity's fixed persistence mode.
    pub fn mode(&self) -> EntityMode {
        self.mode
    }

    /// Sets an automatic compaction policy for retained monotonic snapshots.
    ///
    /// The policy is evaluated after a successful operation that requests
    /// [`EntityDisposition::Retain`]. It does not override explicit outcomes.
    pub fn with_compaction_policy(mut self, policy: CompactionPolicy) -> Self {
        self.compaction_policy = policy;
        self
    }

    /// Append one compact JSON record and a newline to the active generation.
    ///
    /// The returned ID hashes `event_key` and the serialized event. Appending
    /// the same pair again produces the same ID but still appends another
    /// record; use a higher-level idempotency primitive when needed.
    pub fn append(&self, event_key: &str, event: &E::Event) -> Result<JournalEntryId> {
        let event_bytes = serde_json::to_vec(event)?;
        let id = JournalEntryId(sha256_bytes(&[
            event_key.as_bytes(),
            b"\0",
            event_bytes.as_slice(),
        ]));
        let record = EntityRecord {
            id: id.as_str().to_string(),
            event_key: event_key.to_string(),
            event,
        };
        let mut bytes = serde_json::to_vec(&record)?;
        bytes.push(b'\n');

        let _lock = FileLock::exclusive(&self.directory.join("append.lock"))?;
        let active = self.ensure_active_generation()?;
        let path = self.generation_path(&active.id);
        recover_ndjson_tail(&path)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&bytes)?;
        file.sync_data()?;
        Ok(id)
    }

    /// Aggregate the checkpoint and pending NDJSON generations, vend the
    /// interpreted state to a closure, then apply its exact disposition.
    ///
    /// The closure runs while the consumer snapshot is exclusively locked.
    /// Its [`EntityOutcome`] is persisted before the value is returned.
    pub fn with_entity<T>(
        &self,
        operation: impl FnOnce(&EntityView<'_, E>) -> Result<EntityOutcome<T>>,
    ) -> Result<T> {
        self.try_with_entity(operation)
            .map_err(|error| match error {
                EntityOperationError::State(error) | EntityOperationError::Operation(error) => {
                    error
                }
            })
    }

    /// Variant of [`Self::with_entity`] that preserves a client closure's
    /// domain error separately from storage failures.
    ///
    /// If the closure returns an error, no disposition transition is applied;
    /// pending generations remain available to a later consumer.
    pub fn try_with_entity<T, OperationError>(
        &self,
        operation: impl FnOnce(
            &EntityView<'_, E>,
        ) -> std::result::Result<EntityOutcome<T>, OperationError>,
    ) -> std::result::Result<T, EntityOperationError<OperationError>> {
        let batch = self.snapshot_generations()?;
        let checkpoint = self.read_checkpoint()?;
        let cache = self.read_cache()?;
        let covered = batch.generations.iter().cloned().collect::<HashSet<_>>();
        let usable_cache = cache.filter(|cache| {
            cache.checkpoint_revision == checkpoint.revision
                && cache
                    .generations
                    .iter()
                    .all(|generation| covered.contains(generation))
        });
        let (mut state, cached_generations) = usable_cache
            .map(|cache| {
                (
                    cache.state,
                    cache.generations.into_iter().collect::<HashSet<_>>(),
                )
            })
            .unwrap_or_else(|| (checkpoint.state.clone(), HashSet::new()));

        let mut cached_records = Vec::new();
        let mut newly_applied = Vec::new();
        for generation in &batch.generations {
            let generation_records = batch
                .records
                .iter()
                .filter(|record| record.generation == *generation);
            for generated in generation_records {
                if !cached_generations.contains(generation) {
                    state.apply(&generated.record.event);
                    newly_applied.push(generated.record.clone());
                } else {
                    cached_records.push(generated.record.clone());
                }
            }
        }
        // Present cached-source records first so new_events is a contiguous
        // suffix.
        let new_event_offset = cached_records.len();
        let mut ordered_records = cached_records;
        ordered_records.extend(newly_applied);
        let view = EntityView {
            state: &state,
            events: &ordered_records,
            new_event_offset,
            generations: &batch.generations,
        };
        let outcome = operation(&view).map_err(EntityOperationError::Operation)?;
        let disposition = if outcome.disposition == EntityDisposition::Retain
            && self.mode == EntityMode::Monotonic
            && self
                .compaction_policy
                .should_compact(batch.records.len(), batch.bytes)
        {
            EntityDisposition::Compact
        } else {
            outcome.disposition
        };

        match disposition {
            EntityDisposition::Retain => self.write_cache(ProjectionCache {
                checkpoint_revision: checkpoint.revision,
                generations: batch.generations.clone(),
                state,
            })?,
            EntityDisposition::Acknowledge => {
                if self.mode == EntityMode::Monotonic {
                    return Err(StateError::EntityConfiguration(
                        "monotonic entities must compact, not acknowledge, covered events"
                            .to_string(),
                    )
                    .into());
                }
                self.apply_transition(Transition::<E> {
                    kind: TransitionKind::Acknowledge,
                    generations: batch.generations.clone(),
                    checkpoint: None,
                })?;
            }
            EntityDisposition::Compact => self.apply_transition(Transition {
                kind: TransitionKind::Compact,
                generations: batch.generations.clone(),
                checkpoint: Some(Checkpoint {
                    revision: checkpoint.revision.saturating_add(1),
                    state,
                }),
            })?,
        }
        Ok(outcome.value)
    }

    fn snapshot_generations(&self) -> Result<GenerationBatch<E::Event>> {
        let consumer_lock = FileLock::exclusive(&self.directory.join("consumer.lock"))?;
        self.recover_transition()?;
        let _append_lock = FileLock::exclusive(&self.directory.join("append.lock"))?;
        let active = self.ensure_active_generation()?;
        let active_path = self.generation_path(&active.id);
        recover_ndjson_tail(&active_path)?;
        if active_path
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            > 0
        {
            self.write_active_generation(&ActiveGeneration { id: unique_id() })?;
        }
        let active = self.ensure_active_generation()?;
        let mut paths = generation_paths(&self.directory.join("generations"))?;
        paths.retain(|path| path.file_stem().and_then(|value| value.to_str()) != Some(&active.id));

        let mut generations = Vec::with_capacity(paths.len());
        let mut records = Vec::new();
        let mut bytes = 0;
        for path in paths {
            let generation = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| StateError::InvalidRelativePath(path.clone()))?
                .to_string();
            bytes += path.metadata()?.len();
            for record in read_ndjson(&path)? {
                records.push(GeneratedRecord {
                    generation: generation.clone(),
                    record,
                });
            }
            generations.push(generation);
        }
        Ok(GenerationBatch {
            _consumer_lock: consumer_lock,
            generations,
            records,
            bytes,
        })
    }

    fn ensure_active_generation(&self) -> Result<ActiveGeneration> {
        let path = self.directory.join("active-generation.json");
        if path.exists() {
            return Ok(serde_json::from_slice(&std::fs::read(path)?)?);
        }
        let active = ActiveGeneration { id: unique_id() };
        self.write_active_generation(&active)?;
        Ok(active)
    }

    fn write_active_generation(&self, active: &ActiveGeneration) -> Result<()> {
        atomic_replace(
            &self.directory.join("active-generation.json"),
            &serde_json::to_vec_pretty(active)?,
        )
    }

    fn generation_path(&self, id: &str) -> PathBuf {
        self.directory
            .join("generations")
            .join(format!("{id}.ndjson"))
    }

    fn read_checkpoint(&self) -> Result<Checkpoint<E>> {
        let path = self.directory.join("checkpoint.json");
        if !path.exists() {
            return Ok(Checkpoint {
                revision: 0,
                state: E::empty(),
            });
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    fn read_cache(&self) -> Result<Option<ProjectionCache<E>>> {
        let path = self.directory.join("projection-cache.json");
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(serde_json::from_slice(&std::fs::read(path)?)?))
    }

    fn write_cache(&self, cache: ProjectionCache<E>) -> Result<()> {
        atomic_replace(
            &self.directory.join("projection-cache.json"),
            &serde_json::to_vec_pretty(&cache)?,
        )
    }

    fn recover_transition(&self) -> Result<()> {
        let path = self.directory.join("transition.json");
        if !path.exists() {
            return Ok(());
        }
        let transition: Transition<E> = serde_json::from_slice(&std::fs::read(path)?)?;
        self.finish_transition(&transition)
    }

    fn apply_transition(&self, transition: Transition<E>) -> Result<()> {
        atomic_replace(
            &self.directory.join("transition.json"),
            &serde_json::to_vec_pretty(&transition)?,
        )?;
        self.finish_transition(&transition)
    }

    fn finish_transition(&self, transition: &Transition<E>) -> Result<()> {
        for generation in &transition.generations {
            match std::fs::remove_file(self.generation_path(generation)) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        sync_directory(&self.directory.join("generations"))?;
        if let Some(checkpoint) = &transition.checkpoint {
            atomic_replace(
                &self.directory.join("checkpoint.json"),
                &serde_json::to_vec_pretty(checkpoint)?,
            )?;
        }
        remove_if_present(&self.directory.join("projection-cache.json"))?;
        remove_if_present(&self.directory.join("transition.json"))?;
        sync_directory(&self.directory)?;
        Ok(())
    }
}

impl CompactionPolicy {
    fn should_compact(self, entries: usize, bytes: u64) -> bool {
        match self {
            Self::Manual => false,
            Self::AfterEntries(threshold) => threshold > 0 && entries >= threshold,
            Self::AfterBytes(threshold) => threshold > 0 && bytes >= threshold,
        }
    }
}

#[derive(Debug)]
struct GeneratedRecord<T> {
    generation: String,
    record: EntityRecord<T>,
}

struct FileLock {
    file: File,
}

impl FileLock {
    fn exclusive(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn generation_paths(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("ndjson"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn recover_ndjson_tail(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.last() == Some(&b'\n') {
        return Ok(());
    }
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    file.set_len(complete as u64)?;
    file.seek(SeekFrom::Start(complete as u64))?;
    file.sync_data()?;
    Ok(())
}

fn read_ndjson<T: DeserializeOwned>(path: &Path) -> Result<Vec<EntityRecord<T>>> {
    let bytes = std::fs::read(path)?;
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| Ok(serde_json::from_slice(line)?))
        .collect()
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
#[serde(bound(
    serialize = "T: Serialize + Ord",
    deserialize = "T: Deserialize<'de> + Ord"
))]
/// Sorted-set projection for [`SetEvent`] journals.
pub struct SetAggregate<T> {
    values: BTreeSet<T>,
}

impl<T> SetAggregate<T> {
    /// Returns the deterministically ordered projected values.
    pub fn values(&self) -> &BTreeSet<T> {
        &self.values
    }

    /// Reports whether the projected set contains `value`.
    pub fn contains(&self, value: &T) -> bool
    where
        T: Ord,
    {
        self.values.contains(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Append-only event for [`SetAggregate`].
pub enum SetEvent<T> {
    /// Inserts a value; applying an existing value is idempotent.
    Insert(T),
}

impl<T> JournalEntity for SetAggregate<T>
where
    T: Serialize + DeserializeOwned + Clone + Ord,
{
    type Event = SetEvent<T>;

    fn empty() -> Self {
        Self {
            values: BTreeSet::new(),
        }
    }

    fn apply(&mut self, event: &Self::Event) {
        match event {
            SetEvent::Insert(value) => {
                self.values.insert(value.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Result of an atomic [`SetJournal::insert_once`] operation.
pub enum InsertResult {
    /// The value was absent and an insert event was appended.
    Inserted,
    /// The value was already present and no event was appended.
    AlreadyPresent,
}

#[derive(Debug, Clone)]
/// Convenience wrapper around an entity projected as a sorted set.
pub struct SetJournal<T>
where
    T: Serialize + DeserializeOwned + Clone + Ord,
{
    entity: EntityJournal<SetAggregate<T>>,
}

impl<T> SetJournal<T>
where
    T: Serialize + DeserializeOwned + Clone + Ord,
{
    pub(crate) fn new(entity: EntityJournal<SetAggregate<T>>) -> Self {
        Self { entity }
    }

    /// Appends an insert event and returns its content-derived journal ID.
    pub fn insert(&self, event_key: &str, value: T) -> Result<JournalEntryId> {
        self.entity.append(event_key, &SetEvent::Insert(value))
    }

    /// Atomically decide whether a monotonic set value is new, journaling it
    /// before returning `Inserted`.
    pub fn insert_once(&self, event_key: &str, value: T) -> Result<InsertResult> {
        self.entity.with_entity(|view| {
            if view.state().contains(&value) {
                Ok(EntityOutcome::retain(InsertResult::AlreadyPresent))
            } else {
                self.entity
                    .append(event_key, &SetEvent::Insert(value.clone()))?;
                Ok(EntityOutcome::retain(InsertResult::Inserted))
            }
        })
    }

    /// Reports whether the current projected set contains `value`.
    pub fn contains_current(&self, value: &T) -> Result<bool> {
        self.entity
            .with_entity(|view| Ok(EntityOutcome::retain(view.state().contains(value))))
    }

    /// Returns a clone of the current projected set.
    pub fn current(&self) -> Result<BTreeSet<T>> {
        self.entity
            .with_entity(|view| Ok(EntityOutcome::retain(view.state().values().clone())))
    }

    /// Runs `operation` against the current projected set under the entity's
    /// consumer lock while retaining pending generations.
    pub fn with_current<R>(&self, operation: impl FnOnce(&BTreeSet<T>) -> R) -> Result<R> {
        self.entity
            .with_entity(|view| Ok(EntityOutcome::retain(operation(view.state().values()))))
    }

    /// Compacts the current set projection into a durable checkpoint.
    pub fn flush(&self) -> Result<()> {
        self.entity.with_entity(|_| Ok(EntityOutcome::compact(())))
    }

    /// Returns the underlying entity journal.
    pub fn entity(&self) -> &EntityJournal<SetAggregate<T>> {
        &self.entity
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
/// Observation that one path was modified by an agent tool call.
pub struct ModifiedFileEvent {
    /// Modified UTF-8 path.
    pub path: Utf8PathBuf,
    /// Optional native hook event name.
    pub event: Option<String>,
    /// Optional native tool-call identifier.
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
/// Monotonic aggregate of distinct modified paths.
pub struct ModifiedFiles(BTreeSet<Utf8PathBuf>);

impl ModifiedFiles {
    /// Returns the deterministically ordered modified paths.
    pub fn paths(&self) -> &BTreeSet<Utf8PathBuf> {
        &self.0
    }
}

impl JournalEntity for ModifiedFiles {
    type Event = ModifiedFileEvent;

    fn empty() -> Self {
        Self(BTreeSet::new())
    }

    fn apply(&mut self, event: &Self::Event) {
        self.0.insert(event.path.clone());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
/// Observation that an instruction/rule file was loaded.
pub struct LoadedRuleEvent {
    /// Loaded UTF-8 path.
    pub path: Utf8PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
/// Monotonic aggregate of distinct loaded rule paths.
pub struct LoadedRules(BTreeSet<Utf8PathBuf>);

impl LoadedRules {
    /// Returns the deterministically ordered loaded rule paths.
    pub fn paths(&self) -> &BTreeSet<Utf8PathBuf> {
        &self.0
    }
}

impl JournalEntity for LoadedRules {
    type Event = LoadedRuleEvent;

    fn empty() -> Self {
        Self(BTreeSet::new())
    }

    fn apply(&mut self, event: &Self::Event) {
        self.0.insert(event.path.clone());
    }
}
