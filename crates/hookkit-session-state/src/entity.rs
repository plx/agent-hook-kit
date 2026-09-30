use crate::storage::{
    Durability, FileLock, IoContext, atomic_replace, create_private_dir_all, decode, entry_exists,
    publish_if_absent, read_optional, remove_if_present, sha256_bytes, sync_directory, unique_id,
    validate_name,
};
use crate::{JournalEntryId, Result, StateError};
use hookkit_core::Utf8PathBuf;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{BTreeSet, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

/// On-disk representation recorded in every entity descriptor.
const REPRESENTATION: &str = "ndjson-generations";

/// Prefix of sequence-numbered generation IDs.
///
/// Earlier builds named generations `{millis}-{pid}-{counter}`, which starts
/// with a digit; the prefix sorts every sequenced generation after them.
const GENERATION_PREFIX: char = 's';

/// Width of the zero-padded generation sequence number.
const GENERATION_SEQUENCE_DIGITS: usize = 20;

/// Sealed-generation count that triggers compaction of a monotonic
/// [`SetJournal`] unless another policy is configured.
const DEFAULT_SET_COMPACTION_GENERATIONS: usize = 32;

/// Chunk size used when scanning backwards for the last complete NDJSON line.
const TAIL_SCAN_CHUNK: u64 = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Validated name and nonzero schema version for a journal entity.
pub struct EntityId {
    name: String,
    version: u32,
}

impl EntityId {
    /// Creates an entity identity.
    ///
    /// `name` must be a lowercase state identifier (ASCII letters, digits,
    /// `.`, `_`, and `-`) and `version` must be nonzero.
    pub fn new(name: impl Into<String>, version: u32) -> Result<Self> {
        let name = name.into();
        validate_name(&name)?;
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
#[non_exhaustive]
/// Persistence semantics for an entity's aggregate projection.
pub enum EntityMode {
    /// A work window. Acknowledgement removes the aggregate contribution of
    /// the covered generations. Windowed entities cannot be compacted, because
    /// a checkpoint could never be acknowledged away.
    Windowed,
    /// Knowledge accumulated for the life of the entity. Compaction moves
    /// covered events into a durable checkpoint.
    Monotonic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
/// Automatic compaction threshold for a monotonic entity.
pub enum CompactionPolicy {
    /// Compact only when an operation explicitly requests it.
    #[default]
    Manual,
    /// Compact when a snapshot covers at least this many pending entries.
    AfterEntries(usize),
    /// Compact when pending generation files occupy at least this many bytes.
    AfterBytes(u64),
    /// Compact when a snapshot covers at least this many sealed generations.
    ///
    /// Every consumer call seals the generation appended since the previous
    /// one, so this bounds the number of generation files a long-lived
    /// monotonic entity accumulates.
    AfterGenerations(usize),
}

/// Projection contract for a typed aggregate derived from journal events.
pub trait JournalEntity: Serialize + DeserializeOwned + Clone {
    /// Append-only event from which the aggregate is projected.
    type Event: Serialize + DeserializeOwned + Clone;

    /// Creates the projection state used when no checkpoint exists.
    fn empty() -> Self;
    /// Applies one journal event to the projection deterministically.
    ///
    /// Events are applied in append order: generations are sealed with a
    /// monotonically increasing sequence number, and each generation's
    /// records are kept in the order producers appended them.
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
#[non_exhaustive]
/// Persistence action applied after an entity operation succeeds.
pub enum EntityDisposition {
    /// Keep pending generations and cache the current projection.
    Retain,
    /// Remove covered generations without checkpointing them.
    ///
    /// This is valid only for [`EntityMode::Windowed`].
    Acknowledge,
    /// Checkpoint the current projection, then remove covered generations.
    ///
    /// This is valid only for [`EntityMode::Monotonic`].
    Compact,
}

#[derive(Debug)]
/// Value returned by an operation together with its requested disposition.
pub struct EntityOutcome<T> {
    value: T,
    disposition: EntityDisposition,
}

// Deliberately exhaustive: callers split storage failures from their own
// domain errors with a two-arm match.
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

    /// Returns `value` and checkpoints the projected monotonic aggregate.
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
    /// Sequence number of `id`; absent for generations named by older builds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sequence: Option<u64>,
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

struct SealedGeneration<T> {
    id: String,
    records: Vec<EntityRecord<T>>,
}

struct GenerationBatch<T> {
    consumer_lock: FileLock,
    generations: Vec<SealedGeneration<T>>,
    entries: usize,
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
            representation: REPRESENTATION.to_string(),
        };
        // Publishing never replaces an existing descriptor, so of two
        // first-time openers that disagree on the mode, the second one reads
        // the winner's descriptor and fails below instead of overwriting it.
        // On a filesystem without hard links, publication falls back to a
        // replacing rename, so first-time publication is also serialized
        // under a lock that no other operation takes.
        let published = if entry_exists(&descriptor_path)? {
            false
        } else {
            let lock_path = directory.join("descriptor.lock");
            let lock = FileLock::exclusive(&lock_path)?;
            let published =
                publish_if_absent(&descriptor_path, &serde_json::to_vec_pretty(&desired)?);
            let unlock = lock.release(&lock_path);
            let published = published?;
            unlock?;
            published
        };
        if !published {
            let bytes =
                std::fs::read(&descriptor_path).at("read entity descriptor", &descriptor_path)?;
            let actual: Descriptor = decode(&descriptor_path, &bytes)?;
            if actual.mode != mode || actual.representation != desired.representation {
                return Err(StateError::EntityConfiguration(format!(
                    "entity `{}` v{} was opened with incompatible mode or representation",
                    id.name(),
                    id.version()
                )));
            }
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

    /// Returns the automatic compaction policy.
    pub fn compaction_policy(&self) -> CompactionPolicy {
        self.compaction_policy
    }

    /// Sets an automatic compaction policy for retained monotonic snapshots.
    ///
    /// The policy is evaluated after a successful operation that requests
    /// [`EntityDisposition::Retain`]. It does not override explicit outcomes
    /// and is ignored for windowed entities.
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
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .at("open generation", &path)?;
        file.write_all(&bytes).at("append to generation", &path)?;
        file.sync_data().at("sync generation", &path)?;
        Ok(id)
    }

    /// Aggregate the checkpoint and pending NDJSON generations, vend the
    /// interpreted state to a closure, then apply its exact disposition.
    ///
    /// The closure runs while the consumer snapshot is exclusively locked.
    /// Its [`EntityOutcome`] is persisted before the value is returned.
    /// Producers may keep appending from inside the closure, but consuming the
    /// same entity again there fails with [`StateError::LockReentry`].
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
        let GenerationBatch {
            consumer_lock: _consumer_lock,
            generations,
            entries,
            bytes,
        } = self.snapshot_generations()?;
        let checkpoint = self.read_checkpoint()?;
        let generation_ids = generations
            .iter()
            .map(|generation| generation.id.clone())
            .collect::<Vec<_>>();
        let covered = generation_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let usable_cache = self.read_cache().filter(|cache| {
            cache.checkpoint_revision == checkpoint.revision
                && cache
                    .generations
                    .iter()
                    .all(|generation| covered.contains(generation.as_str()))
        });
        // A usable cache names only covered generations, so equal lengths
        // mean it already describes this exact snapshot.
        let cache_is_current = usable_cache
            .as_ref()
            .is_some_and(|cache| cache.generations.len() == generation_ids.len());
        let (mut state, cached_generations) = match usable_cache {
            Some(cache) => (
                cache.state,
                cache.generations.into_iter().collect::<HashSet<_>>(),
            ),
            None => (checkpoint.state.clone(), HashSet::new()),
        };

        // Present cached-source records first so new_events is a contiguous
        // suffix. A usable cache always covers the oldest pending
        // generations, so this keeps append order.
        let mut ordered_records = Vec::with_capacity(entries);
        let mut newly_applied = Vec::new();
        for generation in generations {
            if cached_generations.contains(&generation.id) {
                ordered_records.extend(generation.records);
            } else {
                for record in &generation.records {
                    state.apply(&record.event);
                }
                newly_applied.extend(generation.records);
            }
        }
        let new_event_offset = ordered_records.len();
        ordered_records.extend(newly_applied);
        let view = EntityView {
            state: &state,
            events: &ordered_records,
            new_event_offset,
            generations: &generation_ids,
        };
        let outcome = operation(&view).map_err(EntityOperationError::Operation)?;
        let disposition = if outcome.disposition == EntityDisposition::Retain
            && self.mode == EntityMode::Monotonic
            && self
                .compaction_policy
                .should_compact(entries, bytes, generation_ids.len())
        {
            EntityDisposition::Compact
        } else {
            outcome.disposition
        };

        match disposition {
            EntityDisposition::Retain => {
                if !generation_ids.is_empty() && !cache_is_current {
                    self.write_cache(ProjectionCache {
                        checkpoint_revision: checkpoint.revision,
                        generations: generation_ids,
                        state,
                    })?;
                }
            }
            EntityDisposition::Acknowledge => {
                if self.mode == EntityMode::Monotonic {
                    return Err(StateError::EntityConfiguration(
                        "monotonic entities must compact, not acknowledge, covered events"
                            .to_string(),
                    )
                    .into());
                }
                if !generation_ids.is_empty() {
                    self.apply_transition(Transition::<E> {
                        kind: TransitionKind::Acknowledge,
                        generations: generation_ids,
                        checkpoint: None,
                    })?;
                }
            }
            EntityDisposition::Compact => {
                if self.mode == EntityMode::Windowed {
                    return Err(StateError::EntityConfiguration(
                        "windowed entities must acknowledge, not compact, covered events; \
                         a checkpoint could never be acknowledged away"
                            .to_string(),
                    )
                    .into());
                }
                if !generation_ids.is_empty() {
                    self.apply_transition(Transition {
                        kind: TransitionKind::Compact,
                        generations: generation_ids,
                        checkpoint: Some(Checkpoint {
                            revision: checkpoint.revision.saturating_add(1),
                            state,
                        }),
                    })?;
                }
            }
        }
        Ok(outcome.value)
    }

    fn snapshot_generations(&self) -> Result<GenerationBatch<E::Event>> {
        let consumer_lock = FileLock::exclusive(&self.directory.join("consumer.lock"))?;
        self.recover_transition()?;
        let sealed_paths = {
            let _append_lock = FileLock::exclusive(&self.directory.join("append.lock"))?;
            let mut active = self.ensure_active_generation()?;
            let active_path = self.generation_path(&active.id);
            recover_ndjson_tail(&active_path)?;
            if file_len(&active_path)? > 0 {
                active = self.next_generation(Some(&active))?;
                self.write_active_generation(&active)?;
            }
            let mut paths = generation_paths(&self.directory.join("generations"))?;
            paths.retain(|path| generation_id(path) != Some(active.id.as_str()));
            paths
        };
        // Producers only ever write the new active generation, and only a
        // consumer holding consumer.lock removes sealed ones, so the sealed
        // files are read without blocking producers.
        let mut generations = Vec::with_capacity(sealed_paths.len());
        let mut entries = 0;
        let mut bytes = 0;
        for path in sealed_paths {
            let id = generation_id(&path)
                .ok_or_else(|| StateError::InvalidRelativePath(path.clone()))?
                .to_string();
            let Some(content) = read_optional(&path)? else {
                continue;
            };
            bytes += content.len() as u64;
            let records = parse_ndjson(&path, &content)?;
            entries += records.len();
            generations.push(SealedGeneration { id, records });
        }
        Ok(GenerationBatch {
            consumer_lock,
            generations,
            entries,
            bytes,
        })
    }

    fn ensure_active_generation(&self) -> Result<ActiveGeneration> {
        let path = self.directory.join("active-generation.json");
        if let Some(bytes) = read_optional(&path)? {
            // An unreadable pointer only loses the name of the generation
            // being filled; that file stays in generations/ and is consumed
            // as an ordinary sealed generation.
            if let Ok(active) = serde_json::from_slice::<ActiveGeneration>(&bytes) {
                return Ok(active);
            }
        }
        let active = self.next_generation(None)?;
        self.write_active_generation(&active)?;
        Ok(active)
    }

    /// Allocates the next generation. Callers hold `append.lock`.
    fn next_generation(&self, current: Option<&ActiveGeneration>) -> Result<ActiveGeneration> {
        let previous = match current.and_then(|active| active.sequence) {
            Some(sequence) => Some(sequence),
            None => generation_paths(&self.directory.join("generations"))?
                .iter()
                .filter_map(|path| generation_id(path).and_then(generation_sequence))
                .max(),
        };
        let sequence = previous.map_or(0, |sequence| sequence.saturating_add(1));
        Ok(ActiveGeneration {
            id: format!(
                "{GENERATION_PREFIX}{sequence:0width$}-{}",
                unique_id(),
                width = GENERATION_SEQUENCE_DIGITS
            ),
            sequence: Some(sequence),
        })
    }

    fn write_active_generation(&self, active: &ActiveGeneration) -> Result<()> {
        atomic_replace(
            &self.directory.join("active-generation.json"),
            &serde_json::to_vec_pretty(active)?,
            Durability::Durable,
        )
    }

    fn generation_path(&self, id: &str) -> PathBuf {
        self.directory
            .join("generations")
            .join(format!("{id}.ndjson"))
    }

    fn read_checkpoint(&self) -> Result<Checkpoint<E>> {
        let path = self.directory.join("checkpoint.json");
        match read_optional(&path)? {
            Some(bytes) => decode(&path, &bytes),
            None => Ok(Checkpoint {
                revision: 0,
                state: E::empty(),
            }),
        }
    }

    /// Reads the disposable projection cache; anything unreadable is ignored
    /// and rebuilt from the checkpoint and pending generations.
    fn read_cache(&self) -> Option<ProjectionCache<E>> {
        let bytes = std::fs::read(self.directory.join("projection-cache.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn write_cache(&self, cache: ProjectionCache<E>) -> Result<()> {
        atomic_replace(
            &self.directory.join("projection-cache.json"),
            &serde_json::to_vec_pretty(&cache)?,
            Durability::Disposable,
        )
    }

    fn recover_transition(&self) -> Result<()> {
        let path = self.directory.join("transition.json");
        let Some(bytes) = read_optional(&path)? else {
            return Ok(());
        };
        let transition: Transition<E> = decode(&path, &bytes)?;
        self.finish_transition(&transition)
    }

    fn apply_transition(&self, transition: Transition<E>) -> Result<()> {
        atomic_replace(
            &self.directory.join("transition.json"),
            &serde_json::to_vec_pretty(&transition)?,
            Durability::Durable,
        )?;
        self.finish_transition(&transition)
    }

    fn finish_transition(&self, transition: &Transition<E>) -> Result<()> {
        for generation in &transition.generations {
            remove_if_present(&self.generation_path(generation))?;
        }
        sync_directory(&self.directory.join("generations"))?;
        if let Some(checkpoint) = &transition.checkpoint {
            atomic_replace(
                &self.directory.join("checkpoint.json"),
                &serde_json::to_vec_pretty(checkpoint)?,
                Durability::Durable,
            )?;
        }
        remove_if_present(&self.directory.join("projection-cache.json"))?;
        remove_if_present(&self.directory.join("transition.json"))?;
        sync_directory(&self.directory)?;
        Ok(())
    }
}

impl CompactionPolicy {
    fn should_compact(self, entries: usize, bytes: u64, generations: usize) -> bool {
        match self {
            Self::Manual => false,
            Self::AfterEntries(threshold) => threshold > 0 && entries >= threshold,
            Self::AfterBytes(threshold) => threshold > 0 && bytes >= threshold,
            Self::AfterGenerations(threshold) => threshold > 0 && generations >= threshold,
        }
    }
}

fn generation_id(path: &Path) -> Option<&str> {
    path.file_stem().and_then(|value| value.to_str())
}

/// Parses the sequence number of an `s<digits>-...` generation ID.
fn generation_sequence(id: &str) -> Option<u64> {
    let digits = id
        .strip_prefix(GENERATION_PREFIX)?
        .get(..GENERATION_SEQUENCE_DIGITS)?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Lists generation files in append order.
///
/// Sequenced IDs are zero-padded, so name order is sequence order; IDs from
/// older builds sort first.
fn generation_paths(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at("list generations", directory),
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("ndjson"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn file_len(path: &Path) -> Result<u64> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error).at("inspect generation", path),
    }
}

/// Truncates an incomplete final line left by an interrupted append.
///
/// Only the last byte is read on the common path; the file is scanned
/// backwards only when that byte is not a newline.
fn recover_ndjson_tail(path: &Path) -> Result<()> {
    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).at("open generation", path),
    };
    let length = file.metadata().at("inspect generation", path)?.len();
    if length == 0 || last_byte(&mut file, length).at("read generation", path)? == b'\n' {
        return Ok(());
    }
    let complete = last_line_end(&mut file, length).at("read generation", path)?;
    file.set_len(complete).at("truncate generation", path)?;
    file.sync_data().at("sync generation", path)?;
    Ok(())
}

fn last_byte(file: &mut File, length: u64) -> std::io::Result<u8> {
    let mut byte = [0_u8; 1];
    file.seek(SeekFrom::Start(length - 1))?;
    file.read_exact(&mut byte)?;
    Ok(byte[0])
}

/// Returns the length of the prefix that ends with the last newline.
fn last_line_end(file: &mut File, length: u64) -> std::io::Result<u64> {
    let mut end = length;
    let mut chunk = Vec::new();
    while end > 0 {
        let start = end.saturating_sub(TAIL_SCAN_CHUNK);
        chunk.resize((end - start) as usize, 0);
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;
        if let Some(position) = chunk.iter().rposition(|byte| *byte == b'\n') {
            return Ok(start + position as u64 + 1);
        }
        end = start;
    }
    Ok(0)
}

/// Decodes the newline-terminated records of one generation.
///
/// An unterminated final line can only come from an append that never
/// returned, so it is ignored; a malformed terminated line is corruption.
fn parse_ndjson<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<Vec<EntityRecord<T>>> {
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    bytes[..complete]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| decode(path, line))
        .collect()
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
#[non_exhaustive]
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
#[non_exhaustive]
/// Result of an atomic [`SetJournal::insert_once`] operation.
pub enum InsertResult {
    /// The value was absent and an insert event was appended.
    Inserted,
    /// The value was already present and no event was appended.
    AlreadyPresent,
}

#[derive(Debug, Clone)]
/// Convenience wrapper around an entity projected as a sorted set.
///
/// A monotonic set compacts itself once a consumer call covers 32 sealed
/// generations, so repeated [`Self::insert_once`] calls do not accumulate one
/// generation file per insert. Use [`Self::with_compaction_policy`] to choose
/// a different policy.
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
        Self {
            entity: entity.with_compaction_policy(CompactionPolicy::AfterGenerations(
                DEFAULT_SET_COMPACTION_GENERATIONS,
            )),
        }
    }

    /// Replaces the automatic compaction policy of a monotonic set.
    pub fn with_compaction_policy(self, policy: CompactionPolicy) -> Self {
        Self {
            entity: self.entity.with_compaction_policy(policy),
        }
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
    ///
    /// `operation` must not consume this set again (for example through
    /// [`Self::contains_current`]); that nested call fails with
    /// [`StateError::LockReentry`].
    pub fn with_current<R>(&self, operation: impl FnOnce(&BTreeSet<T>) -> R) -> Result<R> {
        self.entity
            .with_entity(|view| Ok(EntityOutcome::retain(operation(view.state().values()))))
    }

    /// Compacts the current set projection into a durable checkpoint.
    ///
    /// Only monotonic sets can be compacted; a windowed set fails with
    /// [`StateError::EntityConfiguration`] because its contents must be
    /// acknowledged instead.
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
