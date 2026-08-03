use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Final normal source disposition for one file.
///
/// The declaration order is the aggregation severity order. Operational
/// failures deliberately live outside this relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileStatus {
    /// Every applicable workflow completed without finding issues.
    Clean,
    /// A remedy changed the file and the final check passed.
    AutoFixed,
    /// At least one applicable workflow still reports issues.
    ManualFixesNeeded,
}

impl FileStatus {
    /// Combine independent normal results using worst-wins severity.
    pub fn join(self, other: Self) -> Self {
        self.max(other)
    }
}

/// Result of one authoritative, non-mutating check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckOutcome {
    /// The check found no actionable issues.
    Clean,
    /// The check found actionable issues.
    Issues,
}

/// The command role represented by an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandPhase {
    /// First authoritative check before any remedy.
    InitialCheck,
    /// Automatic repair command.
    Remedy,
    /// Authoritative check after a remedy.
    FinalCheck,
    /// Compatibility command combining multiple semantic roles.
    Combined,
    /// Configuration validation rather than an external tool command.
    Configuration,
}

/// Semantic classification assigned to a durable command artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactClassification {
    /// Command completed without finding issues.
    Clean,
    /// Command completed and found actionable issues.
    Issues,
    /// Command ran but failed operationally.
    Failure,
    /// Command could not be spawned.
    SpawnError,
    /// Workflow configuration prevented execution.
    ConfigurationError,
    /// Result could not be classified more precisely.
    Unclassified,
}

/// A durable command report exposed to summaries and templates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunArtifact {
    /// Stable artifact identifier within the run.
    pub id: String,
    /// Absolute path to the durable artifact.
    pub absolute_path: PathBuf,
    /// Artifact path relative to the run directory.
    pub run_relative_path: PathBuf,
    /// Media type describing the artifact contents.
    pub media_type: String,
    /// Tool associated with the artifact, when applicable.
    pub tool_id: Option<String>,
    /// Workflow associated with the artifact, when applicable.
    pub workflow_id: Option<String>,
    /// Deterministic job associated with the artifact, when applicable.
    pub job_id: Option<String>,
    /// Tool report associated with the artifact, when applicable.
    pub report_id: Option<String>,
    /// Command role represented by the artifact.
    pub phase: CommandPhase,
    /// Semantic classification of the command result.
    pub classification: ArtifactClassification,
    /// Process exit code, when a process was started and exited normally.
    pub exit_code: Option<i32>,
    /// Executable invoked to produce the artifact.
    pub program: Option<String>,
    /// Arguments passed to the executable.
    pub arguments: Vec<String>,
    /// Working directory used for the command.
    pub working_directory: Option<PathBuf>,
    /// Files directly assigned to the command invocation.
    pub files: Vec<PathBuf>,
    /// Files considered candidates for result attribution.
    pub candidate_files: Vec<PathBuf>,
    /// Files observed to change while the command ran.
    pub changed_files: Vec<PathBuf>,
    /// Durable textual artifact contents.
    pub contents: String,
}

/// Stable link from a file result to one tool/workflow report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolReportRef {
    /// Identifier of the referenced tool report.
    pub report_id: String,
    /// Artifacts that support the referenced report.
    pub artifact_ids: Vec<String>,
}

impl ToolReportRef {
    /// Creates a report reference with sorted, deduplicated artifact identifiers.
    pub fn new(
        report_id: impl Into<String>,
        artifact_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut artifact_ids = artifact_ids.into_iter().collect::<Vec<_>>();
        artifact_ids.sort();
        artifact_ids.dedup();
        Self {
            report_id: report_id.into(),
            artifact_ids,
        }
    }
}

/// One tool workflow applied to one deterministic job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolReport {
    /// Stable report identifier within the run.
    pub id: String,
    /// Tool that produced the report.
    pub tool_id: String,
    /// Human-readable tool name.
    pub tool_name: String,
    /// Workflow that produced the report.
    pub workflow_id: String,
    /// Deterministic job that produced the report.
    pub job_id: String,
    /// Files eligible for attribution to this report.
    pub candidate_files: Vec<PathBuf>,
    /// Files observed to change during the workflow.
    pub changed_files: Vec<PathBuf>,
    /// Outcome of the initial authoritative check, when one ran.
    pub initial_check: Option<CheckOutcome>,
    /// Whether the workflow attempted an automatic remedy.
    pub fix_attempted: bool,
    /// Outcome of the final authoritative check, when one ran.
    pub final_check: Option<CheckOutcome>,
    /// Whether a job-level result was conservatively attributed to every candidate.
    pub conservative_attribution: bool,
    /// Durable artifacts supporting this report.
    pub artifact_ids: Vec<String>,
}

impl ToolReport {
    /// Sorts and deduplicates path and artifact collections.
    pub fn normalize(&mut self) {
        sort_paths(&mut self.candidate_files);
        sort_paths(&mut self.changed_files);
        self.artifact_ids.sort();
        self.artifact_ids.dedup();
    }

    /// Returns a stable link to this report and its artifacts.
    pub fn reference(&self) -> ToolReportRef {
        ToolReportRef::new(self.id.clone(), self.artifact_ids.clone())
    }
}

/// One normal per-file result after joining all applicable workflows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileResult {
    /// Absolute or runner-normalized file path.
    pub path: PathBuf,
    /// User-facing normalized display path.
    pub display_path: String,
    /// Deferred reporting group identifier.
    pub group_id: String,
    /// Worst normal outcome across applicable workflows.
    pub status: FileStatus,
    /// Whether the runner changed this file.
    pub changed_by_runner: bool,
    /// Tool reports contributing to this result.
    pub reports: Vec<ToolReportRef>,
}

/// One normal contribution to a per-file aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAssessment {
    /// File receiving this contribution.
    pub path: PathBuf,
    /// User-facing normalized display path.
    pub display_path: String,
    /// Deferred reporting group identifier.
    pub group_id: String,
    /// Normal outcome contributed by one workflow.
    pub status: FileStatus,
    /// Whether the contributing workflow changed this file.
    pub changed_by_runner: bool,
    /// Tool report supporting this contribution, when available.
    pub report: Option<ToolReportRef>,
}

impl FileAssessment {
    /// Creates a file assessment with default display and grouping metadata.
    pub fn new(path: impl Into<PathBuf>, status: FileStatus) -> Self {
        let path = path.into();
        Self {
            display_path: display_path(&path),
            path,
            group_id: "other".into(),
            status,
            changed_by_runner: false,
            report: None,
        }
    }
}

/// Environment, configuration, spawn, or tool failure outside source status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationalProblem {
    /// Stable problem identifier within the run.
    pub id: String,
    /// Tool associated with the problem, when applicable.
    pub tool_id: Option<String>,
    /// Command phase associated with the problem, when applicable.
    pub phase: Option<String>,
    /// Files potentially affected by the problem.
    pub affected_files: Vec<PathBuf>,
    /// Human-readable problem description.
    pub message: String,
    /// Durable artifacts supporting the problem.
    pub artifact_ids: Vec<String>,
}

impl OperationalProblem {
    /// Sorts and deduplicates file and artifact collections.
    pub fn normalize(&mut self) {
        sort_paths(&mut self.affected_files);
        self.artifact_ids.sort();
        self.artifact_ids.dedup();
    }
}

/// A known incompleteness in candidate discovery or scope materialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageGap {
    /// Stable gap identifier within the run.
    pub id: String,
    /// Unresolved target associated with the gap, when available.
    pub target: Option<String>,
    /// Human-readable description of incomplete coverage.
    pub message: String,
    /// Whether the gap was retained for a future deferred attempt.
    pub retained: bool,
}

/// Complete runner-owned semantic result before rendering or native lowering.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredRunResult {
    /// Normal per-file results keyed by path.
    pub files: BTreeMap<PathBuf, FileResult>,
    /// Tool/workflow reports keyed by report identifier.
    pub reports: BTreeMap<String, ToolReport>,
    /// Operational failures keyed by problem identifier.
    pub operational_problems: BTreeMap<String, OperationalProblem>,
    /// Candidate files to which no workflow could be applied conclusively.
    pub uncovered_files: BTreeSet<PathBuf>,
    /// Candidate files deliberately excluded as inapplicable.
    pub not_applicable_files: BTreeSet<PathBuf>,
    /// Candidate-discovery and scope-materialization gaps keyed by identifier.
    pub coverage_gaps: BTreeMap<String, CoverageGap>,
    /// Durable run artifacts keyed by artifact identifier.
    pub artifacts: BTreeMap<String, RunArtifact>,
}

impl DeferredRunResult {
    /// Joins one normal workflow contribution into the per-file result map.
    pub fn record_file(&mut self, assessment: FileAssessment) {
        self.uncovered_files.remove(&assessment.path);
        self.not_applicable_files.remove(&assessment.path);
        match self.files.get_mut(&assessment.path) {
            Some(existing) => {
                existing.status = existing.status.join(assessment.status);
                existing.changed_by_runner |= assessment.changed_by_runner;
                if existing.display_path.is_empty() {
                    existing.display_path = assessment.display_path;
                }
                if existing.group_id == "other" && assessment.group_id != "other" {
                    existing.group_id = assessment.group_id;
                }
                if let Some(report) = assessment.report {
                    sorted_insert(&mut existing.reports, report);
                }
            }
            None => {
                let reports = assessment.report.into_iter().collect();
                self.files.insert(
                    assessment.path.clone(),
                    FileResult {
                        path: assessment.path,
                        display_path: assessment.display_path,
                        group_id: assessment.group_id,
                        status: assessment.status,
                        changed_by_runner: assessment.changed_by_runner,
                        reports,
                    },
                );
            }
        }
    }

    /// Attach a job-level result to every candidate when diagnostics do not
    /// provide exact per-file attribution. Snapshot-discovered writes are also
    /// included, even if they were not original session candidates.
    pub fn record_conservative_report(&mut self, mut report: ToolReport, status: FileStatus) {
        report.normalize();
        let reference = report.reference();
        let changed = report
            .changed_files
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let paths = report
            .candidate_files
            .iter()
            .chain(report.changed_files.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        for path in paths {
            let changed_by_runner = changed.contains(&path);
            let mut assessment = FileAssessment::new(
                path,
                if changed_by_runner {
                    status.join(FileStatus::AutoFixed)
                } else {
                    status
                },
            );
            assessment.changed_by_runner = changed_by_runner;
            assessment.report = Some(reference.clone());
            self.record_file(assessment);
        }
        self.reports.insert(report.id.clone(), report);
    }

    /// Records a normalized operational problem outside normal file status.
    pub fn record_operational_problem(&mut self, mut problem: OperationalProblem) {
        problem.normalize();
        self.operational_problems
            .insert(problem.id.clone(), problem);
    }

    /// Marks a path uncovered unless it already has a conclusive disposition.
    pub fn record_uncovered(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if !self.files.contains_key(&path) && !self.not_applicable_files.contains(&path) {
            self.uncovered_files.insert(path);
        }
    }

    /// Marks a path not applicable and removes competing normal dispositions.
    pub fn record_not_applicable(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.files.remove(&path);
        self.uncovered_files.remove(&path);
        self.not_applicable_files.insert(path);
    }

    /// Records a candidate-discovery or scope-materialization gap.
    pub fn record_coverage_gap(&mut self, gap: CoverageGap) {
        self.coverage_gaps.insert(gap.id.clone(), gap);
    }

    /// Records an artifact after normalizing its path collections.
    pub fn record_artifact(&mut self, mut artifact: RunArtifact) {
        sort_paths(&mut artifact.files);
        sort_paths(&mut artifact.candidate_files);
        sort_paths(&mut artifact.changed_files);
        self.artifacts.insert(artifact.id.clone(), artifact);
    }

    /// Returns whether any file still needs manual fixes.
    pub fn has_manual_fixes(&self) -> bool {
        self.files
            .values()
            .any(|file| file.status == FileStatus::ManualFixesNeeded)
    }

    /// Returns whether the run encountered any operational problem.
    pub fn has_operational_problems(&self) -> bool {
        !self.operational_problems.is_empty()
    }
}

fn sorted_insert<T: Ord>(values: &mut Vec<T>, value: T) {
    match values.binary_search(&value) {
        Ok(_) => {}
        Err(index) => values.insert(index, value),
    }
}

fn sort_paths(paths: &mut Vec<PathBuf>) {
    paths.sort();
    paths.dedup();
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: &str, candidates: &[&str], changed: &[&str]) -> ToolReport {
        ToolReport {
            id: id.into(),
            tool_id: id.into(),
            tool_name: id.into(),
            workflow_id: "workflow".into(),
            job_id: "job".into(),
            candidate_files: candidates.iter().map(PathBuf::from).collect(),
            changed_files: changed.iter().map(PathBuf::from).collect(),
            initial_check: Some(CheckOutcome::Issues),
            fix_attempted: !changed.is_empty(),
            final_check: Some(CheckOutcome::Clean),
            conservative_attribution: candidates.len() > 1,
            artifact_ids: vec![format!("{id}-artifact")],
        }
    }

    #[test]
    fn file_status_join_is_explicit_worst_wins_order() {
        let statuses = [
            FileStatus::Clean,
            FileStatus::AutoFixed,
            FileStatus::ManualFixesNeeded,
        ];
        for left in statuses {
            for right in statuses {
                assert_eq!(left.join(right), left.max(right));
                assert_eq!(left.join(right), right.join(left));
            }
        }
        assert_eq!(
            statuses
                .into_iter()
                .reduce(FileStatus::join)
                .expect("statuses"),
            FileStatus::ManualFixesNeeded
        );
    }

    #[test]
    fn aggregation_is_deterministic_and_keeps_every_report() {
        let build = |reverse: bool| {
            let mut result = DeferredRunResult::default();
            let reports = [
                (
                    report("formatter", &["src/a.rs"], &["src/a.rs"]),
                    FileStatus::AutoFixed,
                ),
                (
                    report("linter", &["src/a.rs"], &[]),
                    FileStatus::ManualFixesNeeded,
                ),
            ];
            let order: &[usize] = if reverse { &[1, 0] } else { &[0, 1] };
            for index in order {
                let (report, status) = &reports[*index];
                result.record_conservative_report(report.clone(), *status);
            }
            result
        };
        let forward = build(false);
        let reverse = build(true);
        assert_eq!(forward, reverse);
        let file = forward.files.get(Path::new("src/a.rs")).expect("file");
        assert_eq!(file.status, FileStatus::ManualFixesNeeded);
        assert!(file.changed_by_runner);
        assert_eq!(file.reports.len(), 2);
    }

    #[test]
    fn conservative_batch_report_is_reused_by_all_candidates() {
        let mut result = DeferredRunResult::default();
        result.record_conservative_report(
            report("batch", &["src/b.rs", "src/a.rs"], &[]),
            FileStatus::ManualFixesNeeded,
        );
        assert_eq!(result.files.len(), 2);
        for file in result.files.values() {
            assert_eq!(file.reports[0].report_id, "batch");
        }
    }

    #[test]
    fn changed_non_candidate_file_is_included() {
        let mut result = DeferredRunResult::default();
        result.record_conservative_report(
            report("workspace", &["src/a.rs"], &["Cargo.lock"]),
            FileStatus::Clean,
        );
        let changed = result
            .files
            .get(Path::new("Cargo.lock"))
            .expect("changed file");
        assert_eq!(changed.status, FileStatus::AutoFixed);
        assert!(changed.changed_by_runner);
    }

    #[test]
    fn operational_problems_do_not_change_normal_status() {
        let mut result = DeferredRunResult::default();
        result.record_conservative_report(report("ok", &["src/a.rs"], &[]), FileStatus::Clean);
        result.record_operational_problem(OperationalProblem {
            id: "missing".into(),
            tool_id: Some("missing-tool".into()),
            phase: Some("check".into()),
            affected_files: vec!["src/a.rs".into()],
            message: "missing executable".into(),
            artifact_ids: Vec::new(),
        });
        assert_eq!(
            result.files[Path::new("src/a.rs")].status,
            FileStatus::Clean
        );
        assert!(result.has_operational_problems());
    }

    #[test]
    fn uncovered_and_deleted_files_are_not_called_clean() {
        let mut result = DeferredRunResult::default();
        result.record_uncovered("README.unknown");
        result.record_not_applicable("deleted.rs");
        assert!(result.files.is_empty());
        assert!(result.uncovered_files.contains(Path::new("README.unknown")));
        assert!(
            result
                .not_applicable_files
                .contains(Path::new("deleted.rs"))
        );
    }
}
