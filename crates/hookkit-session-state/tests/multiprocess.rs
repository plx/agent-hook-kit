//! Cross-process concurrency: every other suite coordinates threads within one
//! process, but hooks for one session run as separate processes.
//!
//! Each parent test re-runs this test binary as child processes that execute
//! [`child_role`] with a role passed through the environment.

use hookkit_core::HarnessId;
use hookkit_session_state::{
    ClaimResult, EntityId, EntityMode, EntityOutcome, FamilyId, SessionIdentity, SessionState,
    SetJournal, StateRoot,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ROLE: &str = "HOOKKIT_SESSION_STATE_CHILD_ROLE";
const ROOT: &str = "HOOKKIT_SESSION_STATE_CHILD_ROOT";
const PRODUCERS: usize = 4;
const EVENTS_PER_PRODUCER: usize = 40;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hookkit-session-multiprocess-{label}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open_state(root: &Path) -> SessionState {
    SessionState::open(
        HarnessId::CODEX,
        SessionIdentity::Session("multiprocess".into()),
        StateRoot::new(root),
    )
    .unwrap()
}

fn pending(root: &Path) -> SetJournal<String> {
    open_state(root)
        .family(FamilyId::new("multiprocess", 1).unwrap())
        .unwrap()
        .session_scope()
        .unwrap()
        .set(EntityId::new("pending", 1).unwrap(), EntityMode::Windowed)
        .unwrap()
}

/// Acknowledges the current window and returns the values it contained.
fn consume(set: &SetJournal<String>) -> Vec<String> {
    set.entity()
        .with_entity(|view| {
            Ok(EntityOutcome::acknowledge(
                view.events()
                    .iter()
                    .map(|record| match record.event() {
                        hookkit_session_state::SetEvent::Insert(value) => value.clone(),
                        _ => unreachable!("only inserts are appended"),
                    })
                    .collect(),
            ))
        })
        .unwrap()
}

fn spawn_child(role: &str, root: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_role", "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .env(ROOT, root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

/// Values a child reported. The test harness prints the test name without a
/// newline before the child's own output, so markers are found anywhere in a
/// line.
fn child_output(child: Child) -> Vec<String> {
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "child failed: {output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| {
            line.split_once("value:")
                .map(|(_, value)| value.to_string())
        })
        .collect()
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for start signal"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Entry point for child processes; a no-op in a normal test run.
#[test]
fn child_role() {
    let (Ok(role), Ok(root)) = (std::env::var(ROLE), std::env::var(ROOT)) else {
        return;
    };
    let root = PathBuf::from(root);
    wait_for(&root.join("start"));
    match role.split_once(':') {
        Some(("produce", producer)) => {
            let set = pending(&root);
            for event in 0..EVENTS_PER_PRODUCER {
                let value = format!("{producer}-{event}");
                set.insert(&value, value.clone()).unwrap();
            }
        }
        Some(("consume", _)) => {
            let set = pending(&root);
            loop {
                let stop = root.join("stop").exists();
                for value in consume(&set) {
                    println!("value:{value}");
                }
                if stop {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Some(("claim", _)) => {
            let claims = open_state(&root)
                .family(FamilyId::new("multiprocess", 1).unwrap())
                .unwrap()
                .claims("once")
                .unwrap();
            if claims.try_claim("shared-key").unwrap() == ClaimResult::Claimed {
                println!("value:claimed");
            }
        }
        _ => panic!("unknown child role `{role}`"),
    }
}

#[test]
fn processes_append_and_consume_a_window_without_loss_or_duplication() {
    let directory = TempDirectory::new("window");
    let root = directory.0.as_path();
    // Create the entity before the race so every child agrees on its mode.
    pending(root);
    let producers = (0..PRODUCERS)
        .map(|producer| spawn_child(&format!("produce:{producer}"), root))
        .collect::<Vec<_>>();
    let consumers = (0..2)
        .map(|consumer| spawn_child(&format!("consume:{consumer}"), root))
        .collect::<Vec<_>>();
    std::fs::write(root.join("start"), b"").unwrap();
    for producer in producers {
        child_output(producer);
    }
    std::fs::write(root.join("stop"), b"").unwrap();
    let mut consumed = consumers
        .into_iter()
        .flat_map(child_output)
        .collect::<Vec<_>>();
    consumed.extend(consume(&pending(root)));

    let expected = (0..PRODUCERS)
        .flat_map(|producer| {
            (0..EVENTS_PER_PRODUCER).map(move |event| format!("{producer}-{event}"))
        })
        .collect::<BTreeSet<_>>();
    let unique = consumed.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(unique, expected, "every appended event is consumed");
    assert_eq!(consumed.len(), expected.len(), "no event is consumed twice");
}

#[test]
fn processes_race_for_a_claim_with_exactly_one_winner() {
    let directory = TempDirectory::new("claim");
    let root = directory.0.as_path();
    open_state(root);
    let children = (0..6)
        .map(|child| spawn_child(&format!("claim:{child}"), root))
        .collect::<Vec<_>>();
    std::fs::write(root.join("start"), b"").unwrap();
    let winners = children
        .into_iter()
        .flat_map(child_output)
        .filter(|value| value == "claimed")
        .count();
    assert_eq!(winners, 1);
}
