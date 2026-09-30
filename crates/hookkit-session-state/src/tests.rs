use super::*;
use hookkit_core::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::io::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Record {
    path: String,
}

/// Aggregate that remembers the exact order in which events were applied.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Sequence(Vec<u32>);

impl JournalEntity for Sequence {
    type Event = u32;

    fn empty() -> Self {
        Self(Vec::new())
    }

    fn apply(&mut self, event: &Self::Event) {
        self.0.push(*event);
    }
}

fn temporary_root(label: &str) -> StateRoot {
    StateRoot::new(std::env::temp_dir().join(format!(
        "hookkit-session-state-test-{label}-{}",
        unique_id()
    )))
}

fn family(root: &StateRoot) -> StateFamily {
    SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("session-1".into()),
        root.clone(),
    )
    .unwrap()
    .family(FamilyId::new("test.family", 1).unwrap())
    .unwrap()
}

fn monotonic_set(root: &StateRoot, name: &str) -> SetJournal<String> {
    family(root)
        .session_scope()
        .unwrap()
        .set::<String>(EntityId::new(name, 1).unwrap(), EntityMode::Monotonic)
        .unwrap()
}

fn generation_files(entity: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(entity.join("generations"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".ndjson"))
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn active_generation_path(entity: &Path) -> PathBuf {
    let active: serde_json::Value =
        serde_json::from_slice(&std::fs::read(entity.join("active-generation.json")).unwrap())
            .unwrap();
    entity
        .join("generations")
        .join(format!("{}.ndjson", active["id"].as_str().unwrap()))
}

/// Runs `operation` on a helper thread and fails instead of hanging the suite
/// when it deadlocks.
fn run_with_timeout<T: Send + 'static>(operation: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(operation());
    });
    receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("operation deadlocked")
}

fn native_context(
    session: &str,
    roots: &[&str],
    transcript: Option<&str>,
    boundary: Option<hookkit_core::SessionBoundaryContext>,
) -> hookkit_core::NativeContext {
    hookkit_core::NativeContext {
        workspace_roots: roots.iter().map(|root| Utf8PathBuf::from(*root)).collect(),
        session_id: hookkit_core::SessionId::new(session).ok(),
        transcript_path: transcript.map(Utf8PathBuf::from),
        session_boundary: boundary,
        ..hookkit_core::NativeContext::default()
    }
}

fn ensure_with(
    root: &StateRoot,
    native: hookkit_core::NativeContext,
) -> Result<(SessionState, SessionMetadata)> {
    let raw = hookkit_core::RawInvocation::parse(b"{}".to_vec()).unwrap();
    let context = hookkit_core::RuntimeContext::new(
        HarnessId::CLAUDE_CODE,
        hookkit_core::SnapshotId::builtin("test"),
        hookkit_core::EventId::builtin(HarnessId::CLAUDE_CODE, "SessionStart"),
        hookkit_core::ContractId::builtin("test"),
        hookkit_core::ResolutionProvenance::TypedStatic,
        &raw,
        native,
        &hookkit_core::DISABLED_DIAGNOSTICS,
    )
    .unwrap();
    let state = SessionState::ensure(&context, root.clone())?;
    let metadata = state.metadata()?;
    Ok((state, metadata))
}

fn internal_directory(state: &SessionState) -> PathBuf {
    state.directory().join("_hookkit/metadata/v1")
}

#[test]
fn native_identity_is_hashed_and_harness_scoped() {
    let root = temporary_root("identity");
    let state = SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("secret/session".into()),
        root.clone(),
    )
    .unwrap();
    assert!(!state.directory().to_string_lossy().contains("secret"));
    assert!(state.directory().to_string_lossy().contains("codex"));
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn concurrent_claim_has_one_winner() {
    let root = temporary_root("claim");
    let claims = Arc::new(family(&root).claims("loaded").unwrap());
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let claims = Arc::clone(&claims);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                claims.try_claim("rule-1").unwrap()
            })
        })
        .collect::<Vec<_>>();
    let winners = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|result| *result == ClaimResult::Claimed)
        .count();
    assert_eq!(winners, 1);
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn claim_is_published_complete_without_leftover_temporaries() {
    let root = temporary_root("claim-content");
    let claims = family(&root).claims("once").unwrap();
    assert_eq!(claims.try_claim("key").unwrap(), ClaimResult::Claimed);
    assert_eq!(
        claims.try_claim("key").unwrap(),
        ClaimResult::AlreadyClaimed
    );
    assert!(claims.contains("key"));
    let names = std::fs::read_dir(&claims.directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let claim = format!("{}.claim", sha256("key"));
    assert_eq!(names, vec![claim.clone()]);
    assert_eq!(
        std::fs::read(claims.directory.join(claim)).unwrap(),
        b"claimed\n"
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn unrelated_family_names_have_independent_state() {
    let root = temporary_root("families");
    let state = SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("session-1".into()),
        root.clone(),
    )
    .unwrap();
    let first = state
        .family(FamilyId::new("example.first", 1).unwrap())
        .unwrap()
        .claims("once")
        .unwrap();
    let second = state
        .family(FamilyId::new("example.second", 1).unwrap())
        .unwrap()
        .claims("once")
        .unwrap();
    assert_eq!(first.try_claim("same-key").unwrap(), ClaimResult::Claimed);
    assert_eq!(second.try_claim("same-key").unwrap(), ClaimResult::Claimed);
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn coordination_names_must_be_lowercase() {
    assert!(FamilyId::new("Rules", 1).is_err());
    assert!(EntityId::new("Pending", 1).is_err());
    let root = temporary_root("lowercase");
    let family = family(&root);
    assert!(family.claims("Once").is_err());
    assert!(family.record_journal("Dirty").is_err());
    assert!(family.exclusive_lock("Consumer").is_err());
    assert!(
        family
            .scope(StateScope::Custom {
                kind: "Worker".into(),
                key: "a".into(),
            })
            .is_err()
    );
    // Run labels are prefixed by a unique ID, so either case is safe.
    assert!(family.start_run("Lint").is_ok());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn exclusive_guard_serializes_cooperating_threads() {
    let root = temporary_root("lock");
    let family = Arc::new(family(&root));
    let barrier = Arc::new(Barrier::new(8));
    let active = Arc::new(AtomicUsize::new(0));
    let handles = (0..8)
        .map(|_| {
            let family = Arc::clone(&family);
            let barrier = Arc::clone(&barrier);
            let active = Arc::clone(&active);
            std::thread::spawn(move || {
                barrier.wait();
                let _guard = family.exclusive_lock("consumer").unwrap();
                assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                std::thread::yield_now();
                assert_eq!(active.fetch_sub(1, Ordering::SeqCst), 1);
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn nested_family_lock_reports_reentry_instead_of_deadlocking() {
    let root = temporary_root("lock-reentry");
    let state_family = family(&root);
    let (guard, closure, other) = run_with_timeout(move || {
        let _outer = state_family.exclusive_lock("consumer").unwrap();
        (
            state_family.exclusive_lock("consumer").map(drop),
            state_family.with_exclusive_lock("consumer", || Ok(())),
            state_family.exclusive_lock("other").map(drop),
        )
    });
    assert!(
        matches!(guard, Err(StateError::LockReentry(_))),
        "{guard:?}"
    );
    assert!(
        matches!(closure, Err(StateError::LockReentry(_))),
        "{closure:?}"
    );
    assert!(other.is_ok());
    // The outer guard was released when the helper thread finished.
    assert!(
        family(&root)
            .try_exclusive_lock("consumer")
            .unwrap()
            .is_some()
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn nested_entity_consumer_reports_reentry_instead_of_deadlocking() {
    let root = temporary_root("entity-reentry");
    let set = monotonic_set(&root, "values");
    set.insert("a", "a".into()).unwrap();
    let (outer, inner) = run_with_timeout(move || {
        let mut inner = None;
        let outer = set.with_current(|_| {
            inner = Some(set.contains_current(&"a".to_string()));
        });
        (outer, inner.expect("closure ran"))
    });
    assert!(outer.is_ok());
    assert!(
        matches!(inner, Err(StateError::LockReentry(_))),
        "{inner:?}"
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn try_and_deadline_locks_give_up_while_another_holder_waits() {
    let root = temporary_root("lock-deadline");
    let state_family = Arc::new(family(&root));
    let (held_sender, held_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel::<()>();
    let holder = {
        let state_family = Arc::clone(&state_family);
        std::thread::spawn(move || {
            let _guard = state_family.exclusive_lock("shared-work").unwrap();
            held_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        })
    };
    held_receiver.recv().unwrap();
    assert!(
        state_family
            .try_exclusive_lock("shared-work")
            .unwrap()
            .is_none()
    );
    let started = Instant::now();
    assert!(
        state_family
            .exclusive_lock_timeout("shared-work", Duration::from_millis(40))
            .unwrap()
            .is_none()
    );
    assert!(started.elapsed() >= Duration::from_millis(40));
    release_sender.send(()).unwrap();
    holder.join().unwrap();
    assert!(
        state_family
            .exclusive_lock_timeout("shared-work", Duration::from_secs(10))
            .unwrap()
            .is_some()
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn record_journal_acknowledges_only_the_snapshot() {
    let root = temporary_root("journal");
    let journal = family(&root).record_journal("dirty").unwrap();
    journal
        .append(
            "tool-1",
            &Record {
                path: "a.rs".into(),
            },
        )
        .unwrap();
    let batch = journal.snapshot::<Record>().unwrap();
    journal
        .append(
            "tool-2",
            &Record {
                path: "b.rs".into(),
            },
        )
        .unwrap();
    batch.acknowledge().unwrap();
    let remaining = journal.snapshot::<Record>().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining.entries()[0].value().path, "b.rs");
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn record_journal_keeps_an_identical_record_re_appended_after_the_snapshot() {
    let root = temporary_root("journal-reappend");
    let journal = family(&root).record_journal("dirty").unwrap();
    let record = Record {
        path: "a.rs".into(),
    };
    journal.append("path:a.rs", &record).unwrap();
    let batch = journal.snapshot::<Record>().unwrap();
    assert_eq!(batch.len(), 1);
    // The file was modified again while the consumer was working.
    journal.append("path:a.rs", &record).unwrap();
    batch.acknowledge().unwrap();
    let remaining = journal.snapshot::<Record>().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining.entries()[0].value(), &record);
    remaining.acknowledge().unwrap();
    assert!(journal.snapshot::<Record>().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn record_journal_snapshot_isolates_vanished_and_undecodable_records() {
    let root = temporary_root("journal-fragile");
    let journal = family(&root).record_journal("dirty").unwrap();
    journal
        .append(
            "tool-1",
            &Record {
                path: "a.rs".into(),
            },
        )
        .unwrap();
    // A record acknowledged by a peer between listing and reading.
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        journal.directory.join("missing-target"),
        journal.directory.join("0000.json"),
    )
    .unwrap();
    std::fs::write(journal.directory.join("ffff.json"), br#"{"nope":1}"#).unwrap();

    let batch = journal.snapshot::<Record>().unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(batch.undecodable().len(), 1);
    assert_eq!(batch.undecodable()[0].id().as_str(), "ffff");
    assert!(batch.undecodable()[0].error().contains("path"));
    batch.acknowledge().unwrap();

    let batch = journal.snapshot::<Record>().unwrap();
    assert!(batch.is_empty());
    assert_eq!(batch.undecodable().len(), 1);
    batch.acknowledge_including_undecodable().unwrap();
    assert!(!journal.directory.join("ffff.json").exists());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn run_is_discoverable_only_after_summary_commit() {
    let root = temporary_root("run");
    let run = family(&root).start_run("lint").unwrap();
    let run_directory = run.directory().to_path_buf();
    run.write_text("tools/rustfmt/stdout.txt", "changed")
        .unwrap();
    assert!(!run.is_committed());
    assert!(!run_directory.join("summary.json").exists());
    let expected = serde_json::json!({
        "status": "clean",
        "artifacts": ["tools/rustfmt/stdout.txt"]
    });
    let summary = run.commit(&expected).unwrap();
    assert_eq!(summary.file_name().unwrap(), "summary.json");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(summary).unwrap()).unwrap(),
        expected
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn run_bundle_rewrites_replace_earlier_content() {
    let root = temporary_root("run-rewrite");
    let run = family(&root).start_run("lint").unwrap();
    run.write_text("out.txt", "first").unwrap();
    let path = run.write_text("out.txt", "second").unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "second");
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn run_artifact_paths_cannot_escape_the_bundle() {
    let root = temporary_root("run-path-escape");
    let run = family(&root).start_run("lint").unwrap();
    assert!(matches!(
        run.write_text("../escaped.txt", "no"),
        Err(StateError::InvalidRelativePath(_))
    ));
    assert!(matches!(
        run.write_text(root.path().join("escaped.txt"), "no"),
        Err(StateError::InvalidRelativePath(_))
    ));
    assert!(!root.path().join("escaped.txt").exists());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn observations_are_content_addressed() {
    let root = temporary_root("observation");
    let state = SessionState::open(
        HarnessId::CLAUDE_CODE,
        SessionIdentity::Session("s".into()),
        root.clone(),
    )
    .unwrap();
    let first = state
        .observe_topology(&serde_json::json!({"agent": "a"}))
        .unwrap();
    let second = state
        .observe_topology(&serde_json::json!({"agent": "a"}))
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        state.topology_observations::<serde_json::Value>().unwrap(),
        vec![serde_json::json!({"agent": "a"})]
    );
    assert!(
        state
            .lifecycle_observations::<serde_json::Value>()
            .unwrap()
            .is_empty()
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn observation_reads_skip_records_of_other_shapes() {
    let root = temporary_root("observation-shapes");
    let state = SessionState::open(
        HarnessId::CLAUDE_CODE,
        SessionIdentity::Session("s".into()),
        root.clone(),
    )
    .unwrap();
    state.observe_lifecycle(&json!({"agent": "a"})).unwrap();
    let record = Record {
        path: "x.rs".into(),
    };
    state.observe_lifecycle(&record).unwrap();
    assert_eq!(
        state.lifecycle_observations::<Record>().unwrap(),
        vec![record]
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn rejects_unsafe_family_and_run_paths() {
    assert!(FamilyId::new("../escape", 1).is_err());
    let root = temporary_root("unsafe");
    assert!(
        SessionState::open(
            HarnessId::new("../escape").unwrap(),
            SessionIdentity::Session("s".into()),
            root.clone(),
        )
        .is_err()
    );
    let run = family(&root).start_run("lint").unwrap();
    assert!(run.write_text("../outside", "no").is_err());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn io_errors_name_the_failed_operation_and_path() {
    let root = temporary_root("io-context");
    std::fs::write(root.path(), b"not a directory").unwrap();
    let error = SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("s".into()),
        root.clone(),
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains(&root.path().display().to_string()),
        "{message}"
    );
    assert!(message.contains("use state root"), "{message}");
    assert!(error.io_error().is_some());
    let _ = std::fs::remove_file(root.path());

    let root = temporary_root("io-context-family");
    let state = SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("s".into()),
        root.clone(),
    )
    .unwrap();
    std::fs::write(state.directory().join("families"), b"blocked").unwrap();
    let message = state
        .family(FamilyId::new("blocked", 1).unwrap())
        .unwrap_err()
        .to_string();
    assert!(message.contains("create directory"), "{message}");
    assert!(message.contains("families"), "{message}");
    let _ = std::fs::remove_dir_all(root.path());
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn default_root_is_scoped_to_the_effective_user() {
    let root = StateRoot::default();
    assert!(root.per_user);
    assert!(root.path().starts_with(std::env::temp_dir()));
    assert!(
        root.path().ends_with(
            Path::new(&format!("agent-hook-kit-{}", current_uid())).join("session-state")
        ),
        "{}",
        root.path().display()
    );
}

#[cfg(unix)]
#[test]
fn per_user_root_rejects_a_base_owned_by_another_user() {
    if current_uid() == 0 {
        // Every directory is "owned" by root's uid when tests run as root.
        return;
    }
    // The per-user base is the root's parent, and `/` is owned by root on
    // every supported Unix.
    let root = StateRoot {
        path: PathBuf::from("/hookkit-session-state-test"),
        per_user: true,
    };
    let error = SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("s".into()),
        root.clone(),
    )
    .unwrap_err();
    assert!(
        matches!(error, StateError::UnsafeStateRoot { .. }),
        "{error}"
    );
    assert!(!root.path().exists());
}

#[cfg(unix)]
#[test]
fn per_user_root_is_private_and_retightened_when_owned() {
    use std::os::unix::fs::PermissionsExt;
    let parent = temporary_root("per-user").path().to_path_buf();
    std::fs::create_dir_all(&parent).unwrap();
    let base = parent.join("agent-hook-kit-test");
    let root = StateRoot {
        path: base.join("session-state"),
        per_user: true,
    };
    let open = || {
        SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("s".into()),
            root.clone(),
        )
        .unwrap()
    };
    open();
    assert_eq!(mode(&base), 0o700);
    assert_eq!(mode(root.path()), 0o700);
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
    open();
    assert_eq!(mode(&base), 0o700);
    let _ = std::fs::remove_dir_all(parent);
}

#[cfg(unix)]
#[test]
fn explicit_root_permissions_are_left_alone_and_created_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = temporary_root("explicit-mode");
    std::fs::create_dir_all(root.path()).unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let state = SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("s".into()),
        root.clone(),
    )
    .unwrap();
    let family = state.family(FamilyId::new("modes", 1).unwrap()).unwrap();
    family.claims("once").unwrap();
    assert_eq!(mode(root.path()), 0o755);
    for directory in [
        root.path().join("v1"),
        root.path().join("v1/codex"),
        root.path().join("v1/codex/session"),
        state.directory().to_path_buf(),
        state.directory().join("families"),
        family.directory().to_path_buf(),
        family.directory().join("scopes/session/claims/once"),
    ] {
        assert_eq!(mode(&directory), 0o700, "{}", directory.display());
    }
    let _ = std::fs::remove_dir_all(root.path());
}

#[cfg(unix)]
fn backdate_session(directory: &Path, age: Duration) {
    let time = SystemTime::now() - age;
    let backdate = |path: &Path| {
        std::fs::File::open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    };
    for entry in std::fs::read_dir(directory.join("activity")).unwrap() {
        backdate(&entry.unwrap().path());
    }
    backdate(directory);
}

#[cfg(unix)]
#[test]
fn gc_removes_only_stale_sessions_and_sweeps_leftover_trash() {
    let root = temporary_root("gc");
    let open = |session: &str| {
        SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session(session.into()),
            root.clone(),
        )
        .unwrap()
    };
    let stale = open("stale");
    stale
        .family(FamilyId::new("gc.family", 1).unwrap())
        .unwrap();
    // Opened without any family: the library's own stamp keeps it alive.
    let fresh = open("fresh");
    backdate_session(stale.directory(), Duration::from_secs(7200));
    std::fs::create_dir_all(root.path().join(".trash/leftover/nested")).unwrap();

    let report = SessionState::gc(&root, Duration::from_secs(3600)).unwrap();
    assert_eq!(
        (report.scanned, report.removed, report.failed),
        (2, 1, 0),
        "{report:?}"
    );
    assert!(!stale.directory().exists());
    assert!(fresh.directory().exists());
    assert!(read_dirs(&root.path().join(".trash")).unwrap().is_empty());

    // A collected session reopens cleanly.
    assert!(open("stale").metadata().is_ok());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn gc_tolerates_missing_roots_and_sessions_removed_concurrently() {
    let root = temporary_root("gc-race");
    assert_eq!(
        SessionState::gc(&root, Duration::ZERO).unwrap(),
        GcReport::default()
    );
    let missing = root.path().join("v1/codex/session/missing");
    assert!(!collect_session(&missing, SystemTime::now(), &root.path().join(".trash")).unwrap());
}

#[test]
fn open_automatically_materializes_typed_fallback_metadata() {
    let root = temporary_root("metadata-fallback");
    let state = SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("session-1".into()),
        root.clone(),
    )
    .unwrap();
    let metadata = state.metadata().unwrap();
    assert_eq!(metadata.schema_version, 1);
    assert_eq!(metadata.harness, "codex");
    assert_eq!(metadata.harness_id(), Some(HarnessId::CODEX));
    assert_eq!(
        metadata.current_session.kind,
        SessionEpochKind::FirstObservedFallback
    );
    assert_eq!(
        metadata.current_session.started_at.provenance,
        TimestampProvenance::FirstHookObservation
    );
    assert_eq!(
        state.current_session_started_at().unwrap(),
        metadata.current_session.started_at.at
    );
    assert!(state.metadata_path().is_file());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn ensure_uses_native_start_timestamp_and_captures_project_context() {
    let root = temporary_root("metadata-native");
    let timestamp = "2026-07-12T01:02:03Z";
    let native = native_context(
        "session-1",
        &["/repo"],
        Some("/tmp/transcript.json"),
        Some(
            hookkit_core::SessionBoundaryContext::observed(
                hookkit_core::SessionBoundaryKind::Startup,
            )
            .with_native_timestamp(timestamp)
            .with_occurrence_key("native-start-1"),
        ),
    );
    let (_, first_metadata) = ensure_with(&root, native.clone()).unwrap();
    assert_eq!(
        first_metadata.current_session.kind,
        SessionEpochKind::Startup
    );
    assert_eq!(
        first_metadata.current_session.started_at.at,
        UtcTimestamp::parse_rfc3339(timestamp).unwrap()
    );
    assert_eq!(
        first_metadata.current_session.started_at.provenance,
        TimestampProvenance::NativeEventTimestamp
    );
    assert_eq!(
        first_metadata.project.workspace_roots,
        vec![Utf8PathBuf::from("/repo")]
    );

    let (_, second_metadata) = ensure_with(&root, native).unwrap();
    assert_eq!(
        first_metadata.current_session.id,
        second_metadata.current_session.id
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn repeated_ensure_keeps_one_observation_per_project_context() {
    let root = temporary_root("metadata-growth");
    let native = native_context("session-growth", &["/repo"], Some("/t/a.jsonl"), None);
    let (state, _) = ensure_with(&root, native.clone()).unwrap();
    let metadata_identity =
        storage::FileIdentity::of(&std::fs::metadata(state.metadata_path()).unwrap());
    for _ in 0..25 {
        ensure_with(&root, native.clone()).unwrap();
    }
    let workspaces = internal_directory(&state).join("workspaces");
    assert_eq!(json_files(&workspaces).unwrap().len(), 1);
    assert_eq!(
        storage::FileIdentity::of(&std::fs::metadata(state.metadata_path()).unwrap()),
        metadata_identity,
        "unchanged metadata must not be rewritten"
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn project_metadata_follows_the_latest_context() {
    let root = temporary_root("metadata-latest");
    let first = native_context("session-latest", &["/a"], Some("/t/a.jsonl"), None);
    let second = native_context("session-latest", &["/b"], Some("/t/b.jsonl"), None);
    ensure_with(&root, first.clone()).unwrap();
    let (_, metadata) = ensure_with(&root, second).unwrap();
    assert_eq!(
        metadata.project.transcript_path,
        Some(Utf8PathBuf::from("/t/b.jsonl"))
    );
    let (state, metadata) = ensure_with(&root, first).unwrap();
    assert_eq!(
        metadata.project.transcript_path,
        Some(Utf8PathBuf::from("/t/a.jsonl"))
    );
    assert_eq!(
        metadata.project.workspace_roots,
        vec![Utf8PathBuf::from("/a"), Utf8PathBuf::from("/b")]
    );
    assert_eq!(
        json_files(&internal_directory(&state).join("workspaces"))
            .unwrap()
            .len(),
        2
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn legacy_timestamped_project_observations_are_folded() {
    let root = temporary_root("metadata-legacy");
    let current = native_context("session-legacy", &["/repo"], Some("/t/now.jsonl"), None);
    let (state, _) = ensure_with(&root, current.clone()).unwrap();
    let workspaces = internal_directory(&state).join("workspaces");
    // Earlier builds wrote one content-hash-named file per invocation.
    for (second, roots, transcript) in [
        (1, "/legacy", "/t/legacy.jsonl"),
        (2, "/legacy", "/t/legacy.jsonl"),
        (3, "/legacy", "/t/legacy.jsonl"),
        (4, "/other", "/t/other.jsonl"),
    ] {
        let bytes = serde_json::to_vec_pretty(&json!({
            "observedAt": format!("2020-01-01T00:00:0{second}Z"),
            "workspaceRoots": [roots],
            "transcriptPath": transcript,
            "artifactDirectory": null,
        }))
        .unwrap();
        std::fs::write(
            workspaces.join(format!("{}.json", sha256_bytes(&[&bytes]))),
            bytes,
        )
        .unwrap();
    }
    std::fs::write(workspaces.join("undecodable.json"), b"{").unwrap();

    let (_, metadata) = ensure_with(&root, current).unwrap();
    assert_eq!(
        metadata.project.workspace_roots,
        ["/legacy", "/other", "/repo"]
            .map(Utf8PathBuf::from)
            .to_vec()
    );
    assert_eq!(
        metadata.project.transcript_path,
        Some(Utf8PathBuf::from("/t/now.jsonl"))
    );
    let files = json_files(&workspaces).unwrap();
    assert_eq!(
        files.len(),
        4,
        "three keyed contexts plus the undecodable file"
    );
    let legacy = files
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .find(|value| value["transcriptPath"] == "/t/legacy.jsonl")
        .unwrap();
    assert_eq!(legacy["observedAt"], "2020-01-01T00:00:01Z");
    assert_eq!(legacy["lastObservedAt"], "2020-01-01T00:00:03Z");
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn unknown_epoch_kinds_from_newer_builds_are_tolerated() {
    assert_eq!(
        serde_json::from_str::<SessionEpochKind>("\"brand_new\"").unwrap(),
        SessionEpochKind::Unknown
    );
    assert_eq!(
        serde_json::from_str::<SessionEpochKind>("\"fork\"").unwrap(),
        SessionEpochKind::Fork
    );
    assert_eq!(
        serde_json::from_str::<TimestampProvenance>("\"brand_new\"").unwrap(),
        TimestampProvenance::Unknown
    );

    let root = temporary_root("metadata-forward");
    let (state, _) = ensure_with(
        &root,
        native_context(
            "session-forward",
            &["/repo"],
            None,
            Some(hookkit_core::SessionBoundaryContext::observed(
                hookkit_core::SessionBoundaryKind::Startup,
            )),
        ),
    )
    .unwrap();
    let starts = internal_directory(&state).join("starts");
    std::fs::write(
        starts.join("future.json"),
        serde_json::to_vec(&json!({
            "epoch": {
                "id": "future",
                "kind": "some_future_kind",
                "startedAt": {"at": "2030-01-01T00:00:00Z", "provenance": "some_future_provenance"},
                "firstObservedAt": "2030-01-01T00:00:00Z",
            },
            "occurrenceHash": null,
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(starts.join("torn.json"), b"{\"epoch\":").unwrap();

    let reopened = SessionState::open(
        HarnessId::CLAUDE_CODE,
        SessionIdentity::Session("session-forward".into()),
        root.clone(),
    )
    .unwrap();
    let metadata = reopened.metadata().unwrap();
    assert_eq!(metadata.current_session.kind, SessionEpochKind::Unknown);
    assert_eq!(
        metadata.current_session.started_at.provenance,
        TimestampProvenance::Unknown
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn fork_starts_the_forked_conversation() {
    let root = temporary_root("metadata-fork");
    let timestamp = "2026-09-01T10:00:00Z";
    let (_, metadata) = ensure_with(
        &root,
        native_context(
            "forked-session",
            &["/repo"],
            None,
            Some(
                hookkit_core::SessionBoundaryContext::observed(
                    hookkit_core::SessionBoundaryKind::Fork,
                )
                .with_native_timestamp(timestamp),
            ),
        ),
    )
    .unwrap();
    assert_eq!(metadata.current_session.kind, SessionEpochKind::Fork);
    assert_eq!(
        metadata.conversation.started_at.at,
        UtcTimestamp::parse_rfc3339(timestamp).unwrap()
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn start_boundaries_without_occurrence_keys_coalesce_within_the_window() {
    let root = temporary_root("metadata-coalesce");
    let startup = native_context(
        "session-coalesce",
        &["/repo"],
        None,
        Some(hookkit_core::SessionBoundaryContext::observed(
            hookkit_core::SessionBoundaryKind::Startup,
        )),
    );
    let (state, first) = ensure_with(&root, startup.clone()).unwrap();
    for _ in 0..3 {
        let (_, metadata) = ensure_with(&root, startup.clone()).unwrap();
        assert_eq!(metadata.current_session.id, first.current_session.id);
    }
    let starts = internal_directory(&state).join("starts");
    assert_eq!(json_files(&starts).unwrap().len(), 1);

    let clear = native_context(
        "session-coalesce",
        &["/repo"],
        None,
        Some(hookkit_core::SessionBoundaryContext::observed(
            hookkit_core::SessionBoundaryKind::Clear,
        )),
    );
    let (_, metadata) = ensure_with(&root, clear).unwrap();
    assert_eq!(json_files(&starts).unwrap().len(), 2);
    assert_eq!(metadata.current_session.kind, SessionEpochKind::Clear);
    assert_eq!(
        metadata.conversation.started_at,
        first.current_session.started_at
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn out_of_range_timestamps_fail_serialization_instead_of_panicking() {
    let far_future =
        UtcTimestamp::from_system_time(UNIX_EPOCH + Duration::from_secs(400_000_000_000));
    let error = serde_json::to_string(&far_future).unwrap_err();
    assert!(error.to_string().contains("RFC 3339"), "{error}");
}

fn modified_entity(root: &StateRoot) -> EntityJournal<ModifiedFiles> {
    family(root)
        .session_scope()
        .unwrap()
        .entity(
            EntityId::new("modified-files", 1).unwrap(),
            EntityMode::Windowed,
        )
        .unwrap()
}

fn modified(path: &str) -> ModifiedFileEvent {
    ModifiedFileEvent {
        path: Utf8PathBuf::from(path),
        event: Some("PostToolUse".into()),
        tool_call_id: None,
    }
}

#[test]
fn entity_cache_applies_only_new_ndjson_generations() {
    let root = temporary_root("entity-cache");
    let entity = modified_entity(&root);
    entity.append("one", &modified("/repo/a.rs")).unwrap();
    entity
        .with_entity(|view| {
            assert_eq!(view.state().paths().len(), 1);
            assert_eq!(view.new_events().len(), 1);
            Ok(EntityOutcome::retain(()))
        })
        .unwrap();
    entity.append("two", &modified("/repo/b.rs")).unwrap();
    entity
        .with_entity(|view| {
            assert_eq!(view.state().paths().len(), 2);
            assert_eq!(view.events().len(), 2);
            assert_eq!(view.new_events().len(), 1);
            Ok(EntityOutcome::acknowledge(()))
        })
        .unwrap();
    entity
        .with_entity(|view| {
            assert!(view.state().paths().is_empty());
            assert!(view.events().is_empty());
            Ok(EntityOutcome::retain(()))
        })
        .unwrap();
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn window_ack_does_not_consume_events_appended_by_the_consumer() {
    let root = temporary_root("entity-window");
    let entity = modified_entity(&root);
    entity.append("one", &modified("/repo/a.rs")).unwrap();
    entity
        .with_entity(|view| {
            assert_eq!(view.state().paths().len(), 1);
            entity.append("two", &modified("/repo/b.rs"))?;
            Ok(EntityOutcome::acknowledge(()))
        })
        .unwrap();
    entity
        .with_entity(|view| {
            assert_eq!(
                view.state().paths(),
                &BTreeSet::from([Utf8PathBuf::from("/repo/b.rs")])
            );
            Ok(EntityOutcome::retain(()))
        })
        .unwrap();
    let generation = read_dirs(&entity.directory().join("generations")).unwrap_or_default();
    assert!(
        generation.is_empty(),
        "generations are NDJSON files, not dirs"
    );
    assert!(!generation_files(entity.directory()).is_empty());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn interrupted_append_tails_are_discarded() {
    let root = temporary_root("entity-tail");
    let entity = modified_entity(&root);
    entity.append("one", &modified("/repo/a.rs")).unwrap();
    // A producer killed mid-write leaves an unterminated record behind.
    let active = active_generation_path(entity.directory());
    std::fs::OpenOptions::new()
        .append(true)
        .open(&active)
        .unwrap()
        .write_all(br#"{"id":"partial","eventKey":"#)
        .unwrap();
    entity.append("two", &modified("/repo/b.rs")).unwrap();
    let content = std::fs::read_to_string(&active).unwrap();
    assert_eq!(content.lines().count(), 2);
    assert!(content.ends_with('\n') && !content.contains("partial"));

    // A sealed generation from an older build with the same kind of damage.
    std::fs::write(
        entity.directory().join("generations/1700000000000-1-0.ndjson"),
        concat!(
            r#"{"id":"c","eventKey":"legacy","event":{"path":"/repo/c.rs","event":null,"toolCallId":null}}"#,
            "\n",
            r#"{"id":"d","eventKey":"#
        ),
    )
    .unwrap();
    entity
        .with_entity(|view| {
            assert_eq!(
                view.state().paths(),
                &["/repo/a.rs", "/repo/b.rs", "/repo/c.rs"]
                    .map(Utf8PathBuf::from)
                    .into_iter()
                    .collect()
            );
            assert_eq!(view.events().len(), 3);
            Ok(EntityOutcome::acknowledge(()))
        })
        .unwrap();
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn interrupted_acknowledge_is_finished_by_the_next_consumer() {
    let root = temporary_root("entity-ack-recovery");
    let entity = modified_entity(&root);
    entity.append("one", &modified("/repo/a.rs")).unwrap();
    let sealed = entity
        .with_entity(|view| Ok(EntityOutcome::retain(view.generations().to_vec())))
        .unwrap();
    assert_eq!(sealed.len(), 1);
    assert!(entity.directory().join("projection-cache.json").is_file());
    entity.append("two", &modified("/repo/b.rs")).unwrap();
    // The consumer recorded its intent, then died after deleting only some
    // of the covered generations.
    std::fs::write(
        entity.directory().join("transition.json"),
        serde_json::to_vec(&json!({
            "kind": "acknowledge",
            "generations": [sealed[0], "already-deleted"],
            "checkpoint": null,
        }))
        .unwrap(),
    )
    .unwrap();
    entity
        .with_entity(|view| {
            assert_eq!(
                view.state().paths(),
                &BTreeSet::from([Utf8PathBuf::from("/repo/b.rs")])
            );
            assert_eq!(view.events().len(), 1);
            Ok(EntityOutcome::retain(()))
        })
        .unwrap();
    assert!(!entity.directory().join("transition.json").exists());
    assert!(
        !entity
            .directory()
            .join(format!("generations/{}.ndjson", sealed[0]))
            .exists()
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn interrupted_compaction_is_finished_by_the_next_consumer() {
    let root = temporary_root("entity-compact-recovery");
    let set = monotonic_set(&root, "values");
    set.insert("a", "a".into()).unwrap();
    set.insert("b", "b".into()).unwrap();
    let sealed = set
        .entity()
        .with_entity(|view| Ok(EntityOutcome::retain(view.generations().to_vec())))
        .unwrap();
    // The consumer died after deleting the generations but before writing
    // the checkpoint they were folded into.
    for generation in &sealed {
        std::fs::remove_file(
            set.entity()
                .directory()
                .join(format!("generations/{generation}.ndjson")),
        )
        .unwrap();
    }
    std::fs::write(
        set.entity().directory().join("transition.json"),
        serde_json::to_vec(&json!({
            "kind": "compact",
            "generations": sealed,
            "checkpoint": {"revision": 1, "state": ["a", "b"]},
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        set.current().unwrap(),
        BTreeSet::from(["a".to_string(), "b".to_string()])
    );
    let checkpoint: serde_json::Value = serde_json::from_slice(
        &std::fs::read(set.entity().directory().join("checkpoint.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(checkpoint["revision"], 1);
    assert!(!set.entity().directory().join("transition.json").exists());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn stale_or_corrupt_projection_caches_are_rebuilt() {
    let root = temporary_root("entity-stale-cache");
    let set = monotonic_set(&root, "values");
    set.insert("a", "a".into()).unwrap();
    assert_eq!(set.current().unwrap(), BTreeSet::from(["a".to_string()]));
    let cache_path = set.entity().directory().join("projection-cache.json");
    let mut cache: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache_path).unwrap()).unwrap();
    cache["checkpointRevision"] = json!(7);
    cache["state"] = json!(["bogus"]);
    std::fs::write(&cache_path, serde_json::to_vec(&cache).unwrap()).unwrap();
    assert_eq!(set.current().unwrap(), BTreeSet::from(["a".to_string()]));
    std::fs::write(&cache_path, b"{torn").unwrap();
    assert_eq!(set.current().unwrap(), BTreeSet::from(["a".to_string()]));
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn read_only_consumers_do_not_rewrite_the_projection_cache() {
    let root = temporary_root("entity-cache-writes");
    let set = monotonic_set(&root, "values");
    set.insert("a", "a".into()).unwrap();
    set.current().unwrap();
    let cache_path = set.entity().directory().join("projection-cache.json");
    let identity = storage::FileIdentity::of(&std::fs::metadata(&cache_path).unwrap());
    assert!(set.contains_current(&"a".to_string()).unwrap());
    set.current().unwrap();
    assert_eq!(
        storage::FileIdentity::of(&std::fs::metadata(&cache_path).unwrap()),
        identity
    );
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn entity_modes_are_fixed_even_for_concurrent_first_opens() {
    let root = temporary_root("entity-descriptor");
    let scope = family(&root).session_scope().unwrap();
    scope
        .entity::<ModifiedFiles>(EntityId::new("mode", 1).unwrap(), EntityMode::Windowed)
        .unwrap();
    assert!(matches!(
        scope.entity::<ModifiedFiles>(EntityId::new("mode", 1).unwrap(), EntityMode::Monotonic),
        Err(StateError::EntityConfiguration(_))
    ));
    for trial in 0..20 {
        let barrier = Arc::new(Barrier::new(2));
        let handles = [EntityMode::Windowed, EntityMode::Monotonic].map(|mode| {
            let scope = scope.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                scope
                    .entity::<ModifiedFiles>(
                        EntityId::new(format!("race-{trial}"), 1).unwrap(),
                        mode,
                    )
                    .is_ok()
            })
        });
        let opened = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|opened| *opened)
            .count();
        assert_eq!(opened, 1, "trial {trial}");
    }
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn dispositions_must_match_the_entity_mode() {
    let root = temporary_root("entity-dispositions");
    let set = monotonic_set(&root, "values");
    set.insert("a", "a".into()).unwrap();
    assert!(matches!(
        set.entity()
            .with_entity(|_| Ok(EntityOutcome::acknowledge(()))),
        Err(StateError::EntityConfiguration(_))
    ));
    assert_eq!(set.current().unwrap(), BTreeSet::from(["a".to_string()]));

    let windowed = family(&root)
        .session_scope()
        .unwrap()
        .set::<String>(EntityId::new("window", 1).unwrap(), EntityMode::Windowed)
        .unwrap();
    windowed.insert("a", "a".into()).unwrap();
    assert!(matches!(
        windowed.flush(),
        Err(StateError::EntityConfiguration(_))
    ));
    assert!(
        !windowed
            .entity()
            .directory()
            .join("checkpoint.json")
            .exists()
    );
    windowed
        .entity()
        .with_entity(|_| Ok(EntityOutcome::acknowledge(())))
        .unwrap();
    assert!(windowed.current().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn generations_are_consumed_in_append_order() {
    let root = temporary_root("entity-order");
    let entity = family(&root)
        .session_scope()
        .unwrap()
        .entity::<Sequence>(EntityId::new("sequence", 1).unwrap(), EntityMode::Windowed)
        .unwrap();
    // A generation sealed by an older build predates every sequenced one.
    std::fs::write(
        entity
            .directory()
            .join("generations/1700000000000-1-9.ndjson"),
        "{\"id\":\"legacy\",\"eventKey\":\"legacy\",\"event\":100}\n",
    )
    .unwrap();
    for event in 0..12 {
        entity.append(&format!("event-{event}"), &event).unwrap();
        // Seal each event into its own generation.
        entity
            .with_entity(|_| Ok(EntityOutcome::retain(())))
            .unwrap();
    }
    let generations = entity
        .with_entity(|view| {
            let expected = std::iter::once(100).chain(0..12).collect::<Vec<_>>();
            assert_eq!(view.state().0, expected);
            Ok(EntityOutcome::acknowledge(view.generations().to_vec()))
        })
        .unwrap();
    assert_eq!(generations.len(), 13);
    let mut sorted = generations.clone();
    sorted.sort();
    assert_eq!(generations, sorted);
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn monotonic_set_compaction_preserves_state_without_source_events() {
    let root = temporary_root("entity-set");
    let set = monotonic_set(&root, "loaded-rules");
    assert_eq!(
        set.insert_once("rust", "rust.md".into()).unwrap(),
        InsertResult::Inserted
    );
    assert_eq!(
        set.insert_once("rust", "rust.md".into()).unwrap(),
        InsertResult::AlreadyPresent
    );
    set.flush().unwrap();
    assert_eq!(set.current().unwrap(), BTreeSet::from(["rust.md".into()]));
    assert!(set.entity().directory().join("checkpoint.json").is_file());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn repeated_insert_once_compacts_monotonic_sets_automatically() {
    let root = temporary_root("entity-set-growth");
    let set = monotonic_set(&root, "loaded-rules");
    for index in 0..100 {
        assert_eq!(
            set.insert_once(&format!("rule-{index}"), format!("rule-{index}.md"))
                .unwrap(),
            InsertResult::Inserted
        );
        assert!(generation_files(set.entity().directory()).len() <= 33);
    }
    assert_eq!(set.current().unwrap().len(), 100);
    assert!(set.entity().directory().join("checkpoint.json").is_file());
    let _ = std::fs::remove_dir_all(root.path());
}

#[test]
fn monotonic_set_insert_once_has_one_concurrent_winner() {
    let root = temporary_root("entity-set-concurrent");
    let set = Arc::new(monotonic_set(&root, "loaded-rules"));
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let set = Arc::clone(&set);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                set.insert_once("rust", "rust.md".into()).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let winners = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|result| *result == InsertResult::Inserted)
        .count();
    assert_eq!(winners, 1);
    assert_eq!(set.current().unwrap(), BTreeSet::from(["rust.md".into()]));
    let _ = std::fs::remove_dir_all(root.path());
}
