//! Executable semantics for the durable session-state primitives.
//!
//! The filesystem property uses fewer cases than the pure value properties:
//! every case closes and reopens real journals, exercising serialization,
//! generation projection, checkpoint compaction, and durable reconstruction.

use hookkit_core::HarnessId;
use hookkit_session_state::{
    EntityId, EntityMode, FamilyId, SessionIdentity, SessionState, StateRoot, UtcTimestamp,
};
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hookkit-session-properties-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

proptest! {
    /// Property: all timestamps in the ordinary SystemTime/RFC 3339 range make
    /// a lossless three-way round trip: SystemTime → UtcTimestamp → JSON →
    /// UtcTimestamp → SystemTime. Subsecond precision is retained.
    #[test]
    fn utc_timestamps_round_trip_exactly(seconds in 0u64..2_000_000_000, nanos in 0u32..1_000_000_000) {
        let system = UNIX_EPOCH + Duration::new(seconds, nanos);
        let timestamp = UtcTimestamp::from_system_time(system);
        let json = serde_json::to_vec(&timestamp).unwrap();
        let decoded: UtcTimestamp = serde_json::from_slice(&json).unwrap();

        prop_assert_eq!(decoded, timestamp);
        prop_assert_eq!(decoded.as_system_time(), system);
        prop_assert_eq!(decoded.unix_milliseconds(), (seconds as i128) * 1_000 + (nanos as i128) / 1_000_000);
    }

    /// Property: family and entity identifiers share one deliberately narrow
    /// path-safe grammar, while their positive version remains exact.
    #[test]
    fn versioned_identifiers_preserve_path_safe_names(
        name in "[A-Za-z0-9][A-Za-z0-9._-]{0,30}",
        version in 1u32..=u32::MAX,
    ) {
        let family = FamilyId::new(name.clone(), version).unwrap();
        let entity = EntityId::new(name.clone(), version).unwrap();

        prop_assert_eq!(family.name(), name.as_str());
        prop_assert_eq!(entity.name(), name.as_str());
        prop_assert_eq!(family.version(), version);
        prop_assert_eq!(entity.version(), version);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Property: a monotonic set's value is the mathematical set of appended
    /// values, before and after compaction and a complete store reopen. Input
    /// order and duplicate journal events do not affect durable state.
    #[test]
    fn monotonic_sets_survive_compaction_and_reopen(values in prop::collection::vec(any::<String>(), 0..60)) {
        let directory = TempDirectory::new();
        let root = StateRoot::new(directory.path());
        let identity = SessionIdentity::Session("property-session".into());
        let state = SessionState::open(HarnessId::new("property").unwrap(), identity.clone(), root.clone()).unwrap();
        let family = state.family(FamilyId::new("sets", 1).unwrap()).unwrap();
        let set = family.session_scope().unwrap()
            .set::<String>(EntityId::new("values", 1).unwrap(), EntityMode::Monotonic).unwrap();
        for (index, value) in values.iter().enumerate() {
            set.insert(&format!("event-{index}"), value.clone()).unwrap();
        }
        let expected = values.into_iter().collect::<BTreeSet<_>>();
        prop_assert_eq!(set.current().unwrap(), expected.clone());
        set.flush().unwrap();
        drop(set);
        drop(state);

        let reopened = SessionState::open(HarnessId::new("property").unwrap(), identity, root).unwrap();
        let reopened_set = reopened.family(FamilyId::new("sets", 1).unwrap()).unwrap()
            .session_scope().unwrap()
            .set::<String>(EntityId::new("values", 1).unwrap(), EntityMode::Monotonic).unwrap();
        prop_assert_eq!(reopened_set.current().unwrap(), expected);
    }
}
