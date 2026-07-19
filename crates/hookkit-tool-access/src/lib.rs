//! Phase-agnostic, loss-aware file-access evidence for observable tool calls.
//!
//! Static analysis is not a sandbox or audit log. Reports retain raw and
//! lexically resolved paths, roles, scopes, provenance, certainty, and known
//! gaps, but executables may still perform accesses that are not visible in
//! structured input or statically recoverable Bash syntax.

mod call;
mod model;
mod patch;
mod resolver;
mod structured;

pub use call::{JsonRef, ToolCallObservation, ToolCallRef, observe_post_tool, observe_pre_tool};
pub use hookkit_shell::ToolPhase;
pub use model::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessProvenance, AccessScope, AccessSource,
    AccessTarget, PatchOperation, PathBase, PathExpression, StructuredFieldMatch, ToolAccessGap,
    ToolAccessGapReason, ToolAccessReport,
};
pub use resolver::{
    ExactPathPolicy, ResolutionIssuePolicy, ResolvedTargets, SymlinkPolicy, TargetResolutionError,
    TargetResolutionOptions, TargetResolutionReason, UnresolvedTarget, resolve_targets,
};
pub use structured::{StructuredFieldAnalyzer, StructuredFieldConfigError};

use hookkit_common::{PostToolUseInput, PreToolUseInput};
use hookkit_shell::{
    BashAnalyzer, FileAccessAnalyzer, FileAccessCandidate as ShellCandidate,
    FileAccessCertainty as ShellCertainty, FileAccessKind as ShellIntent, FileInferenceContext,
    FileTarget as ShellTarget, FileTargetScope as ShellScope, PathBase as ShellPathBase,
    ShellToolCallMatch, ShellToolProfile,
};

/// Stateless structured, patch, and shell access analyzer.
#[derive(Default)]
pub struct ToolAccessAnalyzer {
    bash: BashAnalyzer,
    shell: FileAccessAnalyzer,
    structured: StructuredFieldAnalyzer,
    shell_profiles: Vec<ShellToolProfile>,
}

impl ToolAccessAnalyzer {
    pub fn new(
        bash: BashAnalyzer,
        shell: FileAccessAnalyzer,
        structured: StructuredFieldAnalyzer,
    ) -> Self {
        Self {
            bash,
            shell,
            structured,
            shell_profiles: Vec::new(),
        }
    }

    /// Registers an exact opt-in shell tool shape. Later registrations are
    /// checked first, matching custom command-semantics precedence.
    pub fn register_shell_profile(&mut self, profile: ShellToolProfile) {
        self.shell_profiles.insert(0, profile);
    }

    pub fn with_shell_profile(mut self, profile: ShellToolProfile) -> Self {
        self.register_shell_profile(profile);
        self
    }

    pub fn with_shell_profiles(
        mut self,
        profiles: impl IntoIterator<Item = ShellToolProfile>,
    ) -> Self {
        for profile in profiles {
            self.register_shell_profile(profile);
        }
        self
    }

    pub fn structured_fields(&self) -> &StructuredFieldAnalyzer {
        &self.structured
    }

    pub fn structured_fields_mut(&mut self) -> &mut StructuredFieldAnalyzer {
        &mut self.structured
    }

    pub fn analyze_pre_tool(&self, input: &PreToolUseInput) -> ToolAccessReport {
        self.analyze_observation(observe_pre_tool(input))
    }

    pub fn analyze_post_tool(&self, input: &PostToolUseInput) -> ToolAccessReport {
        self.analyze_observation(observe_post_tool(input))
    }

    pub fn analyze_observation(&self, observation: ToolCallObservation<'_>) -> ToolAccessReport {
        match observation {
            ToolCallObservation::Call(call) => self.analyze_call(&call),
            ToolCallObservation::Gap(gap) => ToolAccessReport {
                candidates: Vec::new(),
                gaps: vec![gap],
            },
        }
    }

    pub fn analyze_call(&self, call: &ToolCallRef<'_>) -> ToolAccessReport {
        match &call.shell_call {
            ShellToolCallMatch::Matched(shell_call) => self.analyze_shell(call, shell_call),
            ShellToolCallMatch::Malformed(error) => ToolAccessReport {
                candidates: Vec::new(),
                gaps: vec![ToolAccessGap {
                    source: AccessSource::Shell,
                    reason: ToolAccessGapReason::MalformedShellCall(error.clone()),
                }],
            },
            ShellToolCallMatch::NotShell => {
                match self.explicit_shell_call(call) {
                    ShellToolCallMatch::Matched(shell_call) => {
                        return self.analyze_shell(call, &shell_call);
                    }
                    ShellToolCallMatch::Malformed(error) => {
                        return ToolAccessReport {
                            candidates: Vec::new(),
                            gaps: vec![ToolAccessGap {
                                source: AccessSource::Shell,
                                reason: ToolAccessGapReason::MalformedShellCall(error),
                            }],
                        };
                    }
                    ShellToolCallMatch::NotShell => {}
                    _ => {
                        let mut report = ToolAccessReport::default();
                        report.push_gap(
                            AccessSource::Shell,
                            ToolAccessGapReason::UnsupportedShellEvidence,
                        );
                        return report;
                    }
                }
                let mut report = ToolAccessReport::default();
                if patch::is_patch_tool(call.tool_name) {
                    patch::analyze_patch(call, &mut report);
                } else {
                    self.structured.analyze(call, &mut report);
                }
                report
            }
            _ => {
                let mut report = ToolAccessReport::default();
                report.push_gap(
                    AccessSource::Shell,
                    ToolAccessGapReason::UnsupportedShellEvidence,
                );
                report
            }
        }
    }

    fn explicit_shell_call<'a>(&self, call: &ToolCallRef<'a>) -> ShellToolCallMatch<'a> {
        for profile in &self.shell_profiles {
            let matched = match call.tool_input {
                JsonRef::Value(input) => profile.extract_from_value(
                    call.event.clone(),
                    call.phase,
                    call.tool_name,
                    input,
                    call.cwd,
                    None,
                ),
                JsonRef::Object(input) => profile.extract_from_object(
                    call.event.clone(),
                    call.phase,
                    call.tool_name,
                    input,
                    call.cwd,
                    None,
                ),
            };
            if !matches!(matched, ShellToolCallMatch::NotShell) {
                return matched;
            }
        }
        ShellToolCallMatch::NotShell
    }

    fn analyze_shell(
        &self,
        tool_call: &ToolCallRef<'_>,
        shell_call: &hookkit_shell::ShellToolCallRef<'_>,
    ) -> ToolAccessReport {
        let analysis = self.bash.analyze(shell_call.command);
        let workspace_root = tool_call
            .workspace_roots
            .first()
            .map(hookkit_core::Utf8PathBuf::as_path);
        let inference =
            FileInferenceContext::new(shell_call.cwd).with_workspace_root(workspace_root);
        let shell_report = self.shell.infer(&analysis, inference);
        let mut report = ToolAccessReport::default();

        for candidate in &shell_report.candidates {
            match map_shell_candidate(candidate) {
                Some(candidate) => report.candidates.push(candidate),
                None => report.push_gap(
                    AccessSource::Shell,
                    ToolAccessGapReason::UnsupportedShellEvidence,
                ),
            }
        }
        if let Some(analysis) = analysis.analysis() {
            patch::analyze_shell_patches(analysis, shell_call.cwd, &mut report);
        }
        report.gaps.extend(
            shell_report
                .unresolved
                .into_iter()
                .map(|gap| ToolAccessGap {
                    source: AccessSource::Shell,
                    reason: ToolAccessGapReason::ShellUnresolved(gap),
                }),
        );
        report
    }
}

fn map_shell_candidate(candidate: &ShellCandidate) -> Option<AccessCandidate> {
    let target = match &candidate.target {
        ShellTarget::Path { expression, scope } => AccessTarget::Path {
            expression: PathExpression {
                raw: expression.raw.clone(),
                resolved: expression.resolved.clone(),
                base: match expression.base {
                    ShellPathBase::Absolute => PathBase::Absolute,
                    ShellPathBase::InvocationCwd => PathBase::InvocationCwd,
                    ShellPathBase::UnknownAfterDirectoryChange => {
                        PathBase::UnknownAfterDirectoryChange
                    }
                    _ => return None,
                },
            },
            scope: match scope {
                ShellScope::Exact => AccessScope::Exact,
                ShellScope::Descendants => AccessScope::Descendants,
                ShellScope::ExactOrDescendants => AccessScope::ExactOrDescendants,
                ShellScope::Glob => AccessScope::Glob,
                _ => return None,
            },
        },
        ShellTarget::Workspace { root } => AccessTarget::Workspace { root: root.clone() },
        _ => return None,
    };
    let intent = match candidate.access {
        ShellIntent::Read => AccessIntent::Read,
        ShellIntent::Modify => AccessIntent::Modify,
        ShellIntent::ReadModify => AccessIntent::ReadModify,
        ShellIntent::Enumerate => AccessIntent::Enumerate,
        ShellIntent::Delete => AccessIntent::Delete,
        ShellIntent::MoveSource => AccessIntent::MoveSource,
        ShellIntent::MoveDestination => AccessIntent::MoveDestination,
        _ => return None,
    };
    let certainty = match candidate.certainty {
        ShellCertainty::Direct => AccessCertainty::Direct,
        ShellCertainty::Conditional => AccessCertainty::Conditional,
        ShellCertainty::Heuristic => AccessCertainty::Heuristic,
        _ => return None,
    };
    Some(AccessCandidate {
        target,
        intent,
        certainty,
        provenance: AccessProvenance::Shell {
            origin: candidate.origin.clone(),
            command_span: candidate.command_span,
            inferred_by: candidate.inferred_by.clone(),
        },
    })
}
