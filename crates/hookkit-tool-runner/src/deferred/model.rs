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
    Clean,
    AutoFixed,
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
    Clean,
    Issues,
}

/// The command role represented by an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandPhase {
    InitialCheck,
    Remedy,
    FinalCheck,
    Combined,
    Configuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactClassification {
    Clean,
    Issues,
    Failure,
    SpawnError,
    ConfigurationError,
    Unclassified,
}

/// A durable command report exposed to summaries and templates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunArtifact {
    pub id: String,
    pub absolute_path: PathBuf,
    pub run_relative_path: PathBuf,
    pub media_type: String,
    pub tool_id: Option<String>,
    pub workflow_id: Option<String>,
    pub job_id: Option<String>,
    pub report_id: Option<String>,
    pub phase: CommandPhase,
    pub classification: ArtifactClassification,
    pub exit_code: Option<i32>,
    pub program: Option<String>,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub candidate_files: Vec<PathBuf>,
    pub changed_files: Vec<PathBuf>,
    pub contents: String,
}

/// Stable link from a file result to one tool/workflow report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolReportRef {
    pub report_id: String,
    pub artifact_ids: Vec<String>,
}

impl ToolReportRef {
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
    pub id: String,
    pub tool_id: String,
    pub tool_name: String,
    pub workflow_id: String,
    pub job_id: String,
    pub candidate_files: Vec<PathBuf>,
    pub changed_files: Vec<PathBuf>,
    pub initial_check: Option<CheckOutcome>,
    pub fix_attempted: bool,
    pub final_check: Option<CheckOutcome>,
    pub conservative_attribution: bool,
    pub artifact_ids: Vec<String>,
}

impl ToolReport {
    pub fn normalize(&mut self) {
        sort_paths(&mut self.candidate_files);
        sort_paths(&mut self.changed_files);
        self.artifact_ids.sort();
        self.artifact_ids.dedup();
    }

    pub fn reference(&self) -> ToolReportRef {
        ToolReportRef::new(self.id.clone(), self.artifact_ids.clone())
    }
}

/// One normal per-file result after joining all applicable workflows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileResult {
    pub path: PathBuf,
    pub display_path: String,
    pub group_id: String,
    pub status: FileStatus,
    pub changed_by_runner: bool,
    pub reports: Vec<ToolReportRef>,
}

/// One normal contribution to a per-file aggregate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAssessment {
    pub path: PathBuf,
    pub display_path: String,
    pub group_id: String,
    pub status: FileStatus,
    pub changed_by_runner: bool,
    pub report: Option<ToolReportRef>,
}

impl FileAssessment {
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
    pub id: String,
    pub tool_id: Option<String>,
    pub phase: Option<String>,
    pub affected_files: Vec<PathBuf>,
    pub message: String,
    pub artifact_ids: Vec<String>,
}

impl OperationalProblem {
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
    pub id: String,
    pub target: Option<String>,
    pub message: String,
    pub retained: bool,
}

/// Complete runner-owned semantic result before rendering or native lowering.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredRunResult {
    pub files: BTreeMap<PathBuf, FileResult>,
    pub reports: BTreeMap<String, ToolReport>,
    pub operational_problems: BTreeMap<String, OperationalProblem>,
    pub uncovered_files: BTreeSet<PathBuf>,
    pub not_applicable_files: BTreeSet<PathBuf>,
    pub coverage_gaps: BTreeMap<String, CoverageGap>,
    pub artifacts: BTreeMap<String, RunArtifact>,
}

impl DeferredRunResult {
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

    pub fn record_operational_problem(&mut self, mut problem: OperationalProblem) {
        problem.normalize();
        self.operational_problems
            .insert(problem.id.clone(), problem);
    }

    pub fn record_uncovered(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if !self.files.contains_key(&path) && !self.not_applicable_files.contains(&path) {
            self.uncovered_files.insert(path);
        }
    }

    pub fn record_not_applicable(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.files.remove(&path);
        self.uncovered_files.remove(&path);
        self.not_applicable_files.insert(path);
    }

    pub fn record_coverage_gap(&mut self, gap: CoverageGap) {
        self.coverage_gaps.insert(gap.id.clone(), gap);
    }

    pub fn record_artifact(&mut self, mut artifact: RunArtifact) {
        sort_paths(&mut artifact.files);
        sort_paths(&mut artifact.candidate_files);
        sort_paths(&mut artifact.changed_files);
        self.artifacts.insert(artifact.id.clone(), artifact);
    }

    pub fn has_manual_fixes(&self) -> bool {
        self.files
            .values()
            .any(|file| file.status == FileStatus::ManualFixesNeeded)
    }

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
