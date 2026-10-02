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
//! Conditions that keep changing (an agent that edits a file differently on
//! every attempt without fixing it, or alternates between two broken states)
//! never repeat the previous fingerprint. Claude Code overrides a Stop hook
//! after eight consecutive continuations, but Codex and Antigravity have no
//! such cap, so the runner applies the same one: after
//! [`MAX_CONSECUTIVE_BLOCKS`] blocked attempts in one continuation chain the
//! next is allowed to stop, with a note to the user.
//!
//! Suppressed work stays queued, so the next Stop re-checks it.
//!
//! The same runner family also remembers which file-activity reconciliation
//! gaps were already reported in this session ([`ReportedGaps`]). Those gaps
//! describe persistent conditions (an unreadable directory, a scan budget, a
//! root that is not a Git repository), so reconciliation observes them again
//! at every Stop; reporting them once keeps them from re-blocking or
//! re-running every later Stop.

use crate::util::sha256_hex;
use crate::{CoverageGap, DeferredRunResult, FileStatus};
use hookkit_common::TurnCompletionInput;
use hookkit_pkl_config::schema as pkl;
use hookkit_session_state::{
    EntityId, EntityJournal, EntityMode, EntityOutcome, JournalEntity, StateFamily,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const STOP_GUARD_ENTITY: &str = "stop-loop-guard";
const REPORTED_GAPS_ENTITY: &str = "reported-reconciliation-gaps";

/// Blocked attempts allowed in one continuation chain before the next Stop
/// is let through, matching Claude Code's own eight-continuation cap.
pub(crate) const MAX_CONSECUTIVE_BLOCKS: u32 = 8;

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

    /// Whether this Stop may continue the previous attempt's chain of blocks:
    /// always without a native signal, since such a chain ends only when an
    /// attempt is allowed.
    fn continues_chain(self) -> bool {
        self.repeats_may_stop()
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
    /// Blocked attempts in the continuation chain ending with this attempt;
    /// `0` when it did not block.
    #[serde(default)]
    pub consecutive_blocks: u32,
}

/// Reconciliation gap messages already reported in this session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReportedGaps {
    messages: BTreeSet<String>,
}

/// Gap messages one run reported for the first time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReportedGapsRecord {
    /// Newly reported gap messages.
    pub messages: Vec<String>,
    /// Run bundle that reported them.
    pub run_id: String,
}

impl JournalEntity for ReportedGaps {
    type Event = ReportedGapsRecord;

    fn empty() -> Self {
        Self::default()
    }

    fn apply(&mut self, event: &Self::Event) {
        self.messages.extend(event.messages.iter().cloned());
    }
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
    reported_gaps: EntityJournal<ReportedGaps>,
}

impl StopGuardStore {
    pub(crate) fn open(family: &StateFamily) -> hookkit_session_state::Result<Self> {
        let scope = family.session_scope()?;
        let journal = scope.entity(EntityId::new(STOP_GUARD_ENTITY, 1)?, EntityMode::Monotonic)?;
        let reported_gaps = scope.entity(
            EntityId::new(REPORTED_GAPS_ENTITY, 1)?,
            EntityMode::Monotonic,
        )?;
        Ok(Self {
            journal,
            reported_gaps,
        })
    }

    /// The previous recorded attempt, if any.
    pub(crate) fn previous(&self) -> hookkit_session_state::Result<Option<GuardRecord>> {
        self.journal
            .with_entity(|view| Ok(EntityOutcome::retain(view.state().last.clone())))
    }

    pub(crate) fn record(&self, record: &GuardRecord) -> hookkit_session_state::Result<()> {
        self.journal
            .append(&format!("stop-attempt\0{}", record.run_id), record)?;
        self.journal.with_entity(|_| Ok(EntityOutcome::compact(())))
    }

    /// Reconciliation gap messages already reported in this session.
    pub(crate) fn reported_gaps(&self) -> hookkit_session_state::Result<BTreeSet<String>> {
        self.reported_gaps
            .with_entity(|view| Ok(EntityOutcome::retain(view.state().messages.clone())))
    }

    /// Remember the gap messages a committed run reported.
    pub(crate) fn record_reported_gaps(
        &self,
        record: &ReportedGapsRecord,
    ) -> hookkit_session_state::Result<()> {
        if record.messages.is_empty() {
            return Ok(());
        }
        self.reported_gaps
            .append(&format!("reported-gaps\0{}", record.run_id), record)?;
        self.reported_gaps
            .with_entity(|_| Ok(EntityOutcome::compact(())))
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
    /// Whether a block was converted into an allowed stop because the
    /// continuation chain had already blocked [`MAX_CONSECUTIVE_BLOCKS`] times.
    pub continuation_cap_reached: bool,
    /// Blocked attempts in the continuation chain, this one included.
    pub consecutive_blocks: u32,
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
        previous: Option<&GuardRecord>,
        continuation: ContinuationSignal,
    ) -> Self {
        let previous_fingerprint = previous.and_then(|record| record.fingerprint.clone());
        let chain = previous
            .filter(|_| continuation.continues_chain())
            .map_or(0, |record| record.consecutive_blocks);
        let wanted_block = fingerprint.is_some();
        let suppressed_repeat =
            wanted_block && fingerprint == previous_fingerprint && continuation.repeats_may_stop();
        let continuation_cap_reached =
            wanted_block && !suppressed_repeat && chain >= MAX_CONSECUTIVE_BLOCKS;
        let blocked = wanted_block && !suppressed_repeat && !continuation_cap_reached;
        Self {
            wanted_block,
            blocked,
            suppressed_repeat,
            continuation_cap_reached,
            consecutive_blocks: if blocked { chain.saturating_add(1) } else { 0 },
            fingerprint,
            previous_fingerprint,
            continuation,
        }
    }

    /// The loop-guard record for this attempt.
    pub(crate) fn record(&self, run_id: String) -> GuardRecord {
        GuardRecord {
            fingerprint: self.fingerprint.clone(),
            run_id,
            consecutive_blocks: self.consecutive_blocks,
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

    fn blocked_record(fingerprint: &str, consecutive_blocks: u32) -> GuardRecord {
        GuardRecord {
            fingerprint: Some(fingerprint.into()),
            run_id: "run".into(),
            consecutive_blocks,
        }
    }

    #[test]
    fn identical_repeats_stop_only_on_a_continuation() {
        let first = StopDecision::decide(
            Some("f".into()),
            None,
            ContinuationSignal::StopHookActive(false),
        );
        assert!(first.blocked && !first.suppressed_repeat);
        assert_eq!(first.consecutive_blocks, 1);

        let repeat_in_loop = StopDecision::decide(
            Some("f".into()),
            Some(&blocked_record("f", 1)),
            ContinuationSignal::StopHookActive(true),
        );
        assert!(!repeat_in_loop.blocked && repeat_in_loop.suppressed_repeat);
        assert_eq!(repeat_in_loop.consecutive_blocks, 0);

        let fresh_turn = StopDecision::decide(
            Some("f".into()),
            Some(&blocked_record("f", 1)),
            ContinuationSignal::StopHookActive(false),
        );
        assert!(fresh_turn.blocked, "a new turn gets one block again");

        let progress = StopDecision::decide(
            Some("g".into()),
            Some(&blocked_record("f", 1)),
            ContinuationSignal::StopHookActive(true),
        );
        assert!(progress.blocked, "changed conditions block again");
        assert_eq!(progress.consecutive_blocks, 2);
    }

    #[test]
    fn harnesses_without_a_signal_never_repeat_an_identical_block() {
        let repeat = StopDecision::decide(
            Some("f".into()),
            Some(&blocked_record("f", 1)),
            ContinuationSignal::Unavailable,
        );
        assert!(!repeat.blocked && repeat.suppressed_repeat);
        let nothing = StopDecision::decide(
            None,
            Some(&blocked_record("f", 1)),
            ContinuationSignal::Unavailable,
        );
        assert!(!nothing.wanted_block && !nothing.blocked && !nothing.suppressed_repeat);
    }

    /// Codex never caps continuations, so conditions that change on every
    /// attempt (an unfixable rule the agent keeps rewriting, or alternating
    /// between two broken states) would otherwise block forever.
    #[test]
    fn continuation_chains_stop_blocking_after_the_cap() {
        for continuation in [
            ContinuationSignal::StopHookActive(true),
            ContinuationSignal::Unavailable,
        ] {
            let mut previous: Option<GuardRecord> = None;
            let mut blocked = 0;
            for attempt in 0..20u32 {
                let fingerprint = format!("state-{}", attempt % 2);
                let signal = if attempt == 0 {
                    ContinuationSignal::StopHookActive(false)
                } else {
                    continuation
                };
                let decision = StopDecision::decide(Some(fingerprint), previous.as_ref(), signal);
                if !decision.blocked {
                    assert!(decision.continuation_cap_reached, "{continuation:?}");
                    assert!(!decision.suppressed_repeat);
                    break;
                }
                blocked += 1;
                previous = Some(decision.record(format!("run-{attempt}")));
            }
            assert_eq!(blocked, MAX_CONSECUTIVE_BLOCKS, "{continuation:?}");
        }

        // A fresh Claude Code or Codex turn starts a new chain.
        let fresh = StopDecision::decide(
            Some("g".into()),
            Some(&blocked_record("f", MAX_CONSECUTIVE_BLOCKS)),
            ContinuationSignal::StopHookActive(false),
        );
        assert!(fresh.blocked && !fresh.continuation_cap_reached);
        assert_eq!(fresh.consecutive_blocks, 1);
    }

    #[test]
    fn older_guard_records_without_a_chain_count_still_decode() {
        let record: GuardRecord =
            serde_json::from_str(r#"{"fingerprint":"f","runId":"run"}"#).unwrap();
        assert_eq!(record.consecutive_blocks, 0);
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
