//! Loop protection for deferred Stop blocks.
//!
//! A blocked Stop hands the turn back to the agent. When the agent then stops
//! again without changing anything the runner reported, blocking a second
//! time cannot make progress: Claude Code only bounds the loop with its
//! continuation cap, Codex has no cap, and Antigravity re-enters its loop.
//! The runner therefore fingerprints the conditions that make it block (the
//! content of every file that still needs manual fixes, each operational
//! problem, and strict coverage gaps) and remembers the fingerprint of the
//! previous attempt. A repeat of the same fingerprint is allowed to stop:
//!
//! - on Claude Code and Codex only when `stop_hook_active` says this Stop is
//!   itself the continuation of an earlier Stop hook, so a fresh turn still
//!   gets one block for a problem that is still present;
//! - on Antigravity, which reports no continuation signal, whenever the
//!   previous attempt already reported the identical conditions.
//!
//! Suppressed work stays queued, so the next Stop re-checks it.

use crate::util::sha256_hex;
use crate::{CoverageGap, DeferredRunResult, FileStatus};
use hookkit_common::TurnCompletionInput;
use hookkit_pkl_config::schema as pkl;
use hookkit_session_state::{
    EntityId, EntityJournal, EntityMode, EntityOutcome, JournalEntity, StateFamily,
};
use serde::{Deserialize, Serialize};

const STOP_GUARD_ENTITY: &str = "stop-loop-guard";

/// Harness signal that this Stop continues an earlier Stop-hook block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "stopHookActive")]
pub(crate) enum ContinuationSignal {
    /// The native input carries `stop_hook_active` (Claude Code, Codex).
    StopHookActive(bool),
    /// The harness reports no continuation signal (Antigravity).
    Unavailable,
}

impl ContinuationSignal {
    pub(crate) fn from_input(input: &TurnCompletionInput) -> Self {
        let field = match input {
            TurnCompletionInput::Claude(input) => input.field("stop_hook_active"),
            TurnCompletionInput::Codex(input) => input.field("stop_hook_active"),
            _ => return Self::Unavailable,
        };
        Self::StopHookActive(field.and_then(serde_json::Value::as_bool).unwrap_or(false))
    }

    fn repeats_may_stop(self) -> bool {
        match self {
            Self::StopHookActive(active) => active,
            Self::Unavailable => true,
        }
    }
}

/// Last attempt's blocking fingerprint, if it wanted to block.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StopLoopGuard {
    last: Option<GuardRecord>,
}

/// One recorded Stop attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GuardRecord {
    /// Blocking fingerprint, or `None` when the attempt did not want to block.
    pub fingerprint: Option<String>,
    /// Run bundle that recorded the attempt.
    pub run_id: String,
}

impl JournalEntity for StopLoopGuard {
    type Event = GuardRecord;

    fn empty() -> Self {
        Self::default()
    }

    fn apply(&mut self, event: &Self::Event) {
        self.last = Some(event.clone());
    }
}

/// Durable guard state for one native session.
pub(crate) struct StopGuardStore {
    journal: EntityJournal<StopLoopGuard>,
}

impl StopGuardStore {
    pub(crate) fn open(family: &StateFamily) -> hookkit_session_state::Result<Self> {
        let journal = family
            .session_scope()?
            .entity(EntityId::new(STOP_GUARD_ENTITY, 1)?, EntityMode::Monotonic)?;
        Ok(Self { journal })
    }

    pub(crate) fn previous_fingerprint(&self) -> hookkit_session_state::Result<Option<String>> {
        self.journal.with_entity(|view| {
            Ok(EntityOutcome::retain(
                view.state()
                    .last
                    .as_ref()
                    .and_then(|record| record.fingerprint.clone()),
            ))
        })
    }

    pub(crate) fn record(&self, record: &GuardRecord) -> hookkit_session_state::Result<()> {
        self.journal
            .append(&format!("stop-attempt\0{}", record.run_id), record)?;
        self.journal.with_entity(|_| Ok(EntityOutcome::compact(())))
    }
}

/// The recorded Stop decision, including loop-guard provenance.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StopDecision {
    /// Whether the deferred results asked to block completion.
    pub wanted_block: bool,
    /// Whether the native response blocks completion.
    pub blocked: bool,
    /// Whether an identical repeat block was converted into an allowed stop.
    pub suppressed_repeat: bool,
    /// Fingerprint of the blocking conditions, when any exist.
    pub fingerprint: Option<String>,
    /// Fingerprint recorded by the previous attempt, when it wanted to block.
    pub previous_fingerprint: Option<String>,
    /// Native continuation signal reported by this Stop.
    pub continuation: ContinuationSignal,
}

impl StopDecision {
    pub(crate) fn decide(
        fingerprint: Option<String>,
        previous_fingerprint: Option<String>,
        continuation: ContinuationSignal,
    ) -> Self {
        let wanted_block = fingerprint.is_some();
        let suppressed_repeat =
            wanted_block && fingerprint == previous_fingerprint && continuation.repeats_may_stop();
        Self {
            wanted_block,
            blocked: wanted_block && !suppressed_repeat,
            suppressed_repeat,
            fingerprint,
            previous_fingerprint,
            continuation,
        }
    }
}

/// Fingerprint the conditions that make a deferred result block, or `None`
/// when nothing blocks. `blocking_gaps` are the coverage gaps that block under
/// the strict coverage policy.
pub(crate) fn blocking_fingerprint(
    result: &DeferredRunResult,
    coverage_policy: pkl::CoverageGapPolicy,
) -> Option<String> {
    let mut lines = Vec::new();
    for file in result.files.values() {
        if file.status == FileStatus::ManualFixesNeeded {
            let content = std::fs::read(&file.path)
                .map(|bytes| sha256_hex(&bytes))
                .unwrap_or_else(|_| "missing".into());
            lines.push(format!("manual\0{}\0{content}", file.path.display()));
        }
    }
    for problem in result.operational_problems.values() {
        let files = problem
            .affected_files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\0");
        lines.push(format!(
            "operational\0{}\0{}\0{}\0{files}",
            problem.tool_id.as_deref().unwrap_or_default(),
            problem.phase.as_deref().unwrap_or_default(),
            problem.message
        ));
    }
    if coverage_policy == pkl::CoverageGapPolicy::Strict {
        lines.extend(
            result
                .coverage_gaps
                .values()
                .map(|gap: &CoverageGap| format!("gap\0{}", gap.message)),
        );
    }
    if lines.is_empty() {
        return None;
    }
    lines.sort();
    Some(sha256_hex(lines.join("\n").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_repeats_stop_only_on_a_continuation() {
        let first = StopDecision::decide(
            Some("f".into()),
            None,
            ContinuationSignal::StopHookActive(false),
        );
        assert!(first.blocked && !first.suppressed_repeat);

        let repeat_in_loop = StopDecision::decide(
            Some("f".into()),
            Some("f".into()),
            ContinuationSignal::StopHookActive(true),
        );
        assert!(!repeat_in_loop.blocked && repeat_in_loop.suppressed_repeat);

        let fresh_turn = StopDecision::decide(
            Some("f".into()),
            Some("f".into()),
            ContinuationSignal::StopHookActive(false),
        );
        assert!(fresh_turn.blocked, "a new turn gets one block again");

        let progress = StopDecision::decide(
            Some("g".into()),
            Some("f".into()),
            ContinuationSignal::StopHookActive(true),
        );
        assert!(progress.blocked, "changed conditions block again");
    }

    #[test]
    fn harnesses_without_a_signal_never_repeat_an_identical_block() {
        let repeat = StopDecision::decide(
            Some("f".into()),
            Some("f".into()),
            ContinuationSignal::Unavailable,
        );
        assert!(!repeat.blocked && repeat.suppressed_repeat);
        let nothing = StopDecision::decide(None, Some("f".into()), ContinuationSignal::Unavailable);
        assert!(!nothing.wanted_block && !nothing.blocked && !nothing.suppressed_repeat);
    }

    #[test]
    fn best_effort_gaps_do_not_block_and_strict_gaps_do() {
        let mut result = DeferredRunResult::default();
        result.record_coverage_gap(CoverageGap {
            id: "gap".into(),
            target: None,
            message: "dynamic target".into(),
            retained: false,
        });
        assert!(blocking_fingerprint(&result, pkl::CoverageGapPolicy::BestEffort).is_none());
        assert!(blocking_fingerprint(&result, pkl::CoverageGapPolicy::Strict).is_some());
    }
}
