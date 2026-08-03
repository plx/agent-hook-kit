use super::{DeferredRunResult, FileResult, FileStatus};
use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_pkl_config::schema as pkl;
use minijinja::Environment;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const CLEAN_USER: &str = "clean.user";
const CLEAN_AGENT: &str = "clean.agent";
const AUTO_USER: &str = "auto-fixed.user";
const AUTO_AGENT: &str = "auto-fixed.agent";
const MANUAL_USER: &str = "manual.user";
const MANUAL_AGENT: &str = "manual.agent";
const OPERATIONAL_USER: &str = "operational.user";
const OPERATIONAL_AGENT: &str = "operational.agent";
const MASTER_USER: &str = "master.user";
const MASTER_AGENT: &str = "master.agent";

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReportingError {
    #[error("invalid deferred reporting group `{group}` glob `{pattern}`: {message}")]
    InvalidGlob {
        group: String,
        pattern: String,
        message: String,
    },
    #[error("deferred reporting group ids must be nonempty and unique: `{0}`")]
    InvalidGroupId(String),
    #[error("deferred reporting fallback group `other` must be last")]
    OtherNotLast,
    #[error("invalid deferred reporting template `{name}`: {message}")]
    InvalidTemplate { name: String, message: String },
    #[error("could not build deferred reporting context: {0}")]
    Context(String),
    #[error("could not render deferred reporting template `{name}`: {message}")]
    Render { name: String, message: String },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct RenderedPair {
    pub user: String,
    pub agent: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct RenderedBuckets {
    pub clean: RenderedPair,
    pub auto_fixed: RenderedPair,
    pub manual_fixes_needed: RenderedPair,
    pub operational_error: RenderedPair,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct RenderedMessages {
    pub buckets: RenderedBuckets,
    pub user: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug)]
struct CompiledGroup {
    id: String,
    display_name: String,
    matcher: Option<GlobSet>,
}

pub(crate) struct DeferredReporter {
    config: pkl::DeferredReporting,
    groups: Vec<CompiledGroup>,
    templates: Environment<'static>,
}

impl DeferredReporter {
    /// Compile every glob and template before the deferred engine can mutate
    /// files. Runtime rendering errors are still handled as operational
    /// configuration failures by the caller.
    pub(crate) fn new(config: &pkl::DeferredReporting) -> Result<Self, ReportingError> {
        let mut groups = Vec::new();
        let mut ids = BTreeSet::new();
        for (index, group) in config.groups.iter().enumerate() {
            if group.id.is_empty() || !ids.insert(group.id.clone()) {
                return Err(ReportingError::InvalidGroupId(group.id.clone()));
            }
            if group.id == "other" && index + 1 != config.groups.len() {
                return Err(ReportingError::OtherNotLast);
            }
            let matcher = if group.id == "other" {
                None
            } else {
                let mut builder = GlobSetBuilder::new();
                for pattern in &group.include {
                    let glob = Glob::new(pattern).map_err(|error| ReportingError::InvalidGlob {
                        group: group.id.clone(),
                        pattern: pattern.clone(),
                        message: error.to_string(),
                    })?;
                    builder.add(glob);
                }
                Some(
                    builder
                        .build()
                        .map_err(|error| ReportingError::InvalidGlob {
                            group: group.id.clone(),
                            pattern: group.include.join(", "),
                            message: error.to_string(),
                        })?,
                )
            };
            groups.push(CompiledGroup {
                id: group.id.clone(),
                display_name: group.display_name.clone(),
                matcher,
            });
        }
        if !ids.contains("other") {
            groups.push(CompiledGroup {
                id: "other".into(),
                display_name: "Other".into(),
                matcher: None,
            });
        }

        let mut templates = Environment::new();
        for (name, source) in [
            (CLEAN_USER, config.clean.user.clone()),
            (CLEAN_AGENT, config.clean.agent.clone()),
            (AUTO_USER, config.auto_fixed.user.clone()),
            (AUTO_AGENT, config.auto_fixed.agent.clone()),
            (MANUAL_USER, config.manual_fixes_needed.user.clone()),
            (MANUAL_AGENT, config.manual_fixes_needed.agent.clone()),
            (OPERATIONAL_USER, config.operational_error.user.clone()),
            (OPERATIONAL_AGENT, config.operational_error.agent.clone()),
            (MASTER_USER, config.master_user.clone()),
            (MASTER_AGENT, config.master_agent.clone()),
        ] {
            templates
                .add_template_owned(name, source)
                .map_err(|error| ReportingError::InvalidTemplate {
                    name: name.into(),
                    message: error.to_string(),
                })?;
        }
        Ok(Self {
            config: config.clone(),
            groups,
            templates,
        })
    }

    pub(crate) fn apply_groups(&self, result: &mut DeferredRunResult, project_root: &Path) {
        for file in result.files.values_mut() {
            let candidate = file.path.strip_prefix(project_root).unwrap_or(&file.path);
            file.display_path = candidate.to_string_lossy().replace('\\', "/");
            let group = self
                .groups
                .iter()
                .find(|group| {
                    group
                        .matcher
                        .as_ref()
                        .is_some_and(|matcher| matcher.is_match(candidate))
                })
                .or_else(|| self.groups.iter().find(|group| group.id == "other"))
                .expect("reporter always has an other group");
            file.group_id = group.id.clone();
        }
    }

    pub(crate) fn render(
        &self,
        result: &DeferredRunResult,
        run: TemplateRun<'_>,
    ) -> Result<RenderedMessages, ReportingError> {
        let mut context = self.context(result, run)?;
        let counts = context
            .get("counts")
            .and_then(Value::as_object)
            .expect("reporting context counts");
        let count = |name: &str| counts.get(name).and_then(Value::as_u64).unwrap_or(0);
        let render_empty = self.config.render_empty_buckets;
        let buckets = RenderedBuckets {
            clean: self.render_pair(
                CLEAN_USER,
                CLEAN_AGENT,
                render_empty || count("clean") > 0,
                &context,
            )?,
            auto_fixed: self.render_pair(
                AUTO_USER,
                AUTO_AGENT,
                render_empty || count("auto_fixed") > 0,
                &context,
            )?,
            manual_fixes_needed: self.render_pair(
                MANUAL_USER,
                MANUAL_AGENT,
                render_empty || count("manual_fixes_needed") > 0,
                &context,
            )?,
            operational_error: self.render_pair(
                OPERATIONAL_USER,
                OPERATIONAL_AGENT,
                render_empty || count("operational_errors") > 0,
                &context,
            )?,
        };
        let rendered_buckets = serde_json::to_value(&buckets)
            .map_err(|error| ReportingError::Context(error.to_string()))?;
        let user_list = [
            &buckets.clean.user,
            &buckets.auto_fixed.user,
            &buckets.manual_fixes_needed.user,
            &buckets.operational_error.user,
        ]
        .into_iter()
        .filter(|message| !message.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
        let agent_list = [
            &buckets.clean.agent,
            &buckets.auto_fixed.agent,
            &buckets.manual_fixes_needed.agent,
            &buckets.operational_error.agent,
        ]
        .into_iter()
        .filter(|message| !message.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
        let object = context
            .as_object_mut()
            .expect("reporting context is an object");
        object.insert("rendered_buckets".into(), rendered_buckets);
        object.insert(
            "rendered_bucket_lists".into(),
            json!({"user": user_list, "agent": agent_list}),
        );
        let user = nonempty(self.render_template(MASTER_USER, &context)?);
        let agent = nonempty(self.render_template(MASTER_AGENT, &context)?);
        Ok(RenderedMessages {
            buckets,
            user,
            agent,
        })
    }

    fn render_pair(
        &self,
        user_name: &str,
        agent_name: &str,
        render: bool,
        context: &Value,
    ) -> Result<RenderedPair, ReportingError> {
        if !render {
            return Ok(RenderedPair::default());
        }
        Ok(RenderedPair {
            user: self.render_template(user_name, context)?,
            agent: self.render_template(agent_name, context)?,
        })
    }

    fn render_template(&self, name: &str, context: &Value) -> Result<String, ReportingError> {
        self.templates
            .get_template(name)
            .and_then(|template| template.render(context))
            .map_err(|error| ReportingError::Render {
                name: name.into(),
                message: error.to_string(),
            })
    }

    fn context(
        &self,
        result: &DeferredRunResult,
        run: TemplateRun<'_>,
    ) -> Result<Value, ReportingError> {
        let clean_files = files_with_status(result, FileStatus::Clean);
        let auto_fixed_files = files_with_status(result, FileStatus::AutoFixed);
        let manual_fix_files = files_with_status(result, FileStatus::ManualFixesNeeded);
        let mut groups = Vec::new();
        let mut manual_groups = 0usize;
        for configured in &self.groups {
            let files = result
                .files
                .values()
                .filter(|file| file.group_id == configured.id)
                .collect::<Vec<_>>();
            if files.is_empty() {
                continue;
            }
            let manual = files
                .iter()
                .copied()
                .filter(|file| file.status == FileStatus::ManualFixesNeeded)
                .collect::<Vec<_>>();
            if !manual.is_empty() {
                manual_groups += 1;
            }
            let artifact_paths = associated_artifact_paths(result, &manual);
            groups.push(json!({
                "id": configured.id,
                "display_name": configured.display_name,
                "files": files,
                "count": files.len(),
                "manual_fix_files": manual,
                "artifact_paths": artifact_paths,
            }));
        }
        let artifact_paths = result
            .artifacts
            .values()
            .map(|artifact| artifact.absolute_path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let artifact_contents = result
            .artifacts
            .values()
            .map(|artifact| {
                (
                    artifact.absolute_path.to_string_lossy().into_owned(),
                    artifact.contents.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        serde_json::to_value(json!({
            "run": {
                "id": run.id,
                "project_root": run.project_root,
                "summary_path": run.summary_path,
                "state_directory": run.state_directory,
            },
            "counts": {
                "clean": clean_files.len(),
                "auto_fixed": auto_fixed_files.len(),
                "manual_fixes_needed": manual_fix_files.len(),
                "manual_groups": manual_groups,
                "operational_errors": result.operational_problems.len(),
                "uncovered": result.uncovered_files.len(),
                "not_applicable": result.not_applicable_files.len(),
                "coverage_gaps": result.coverage_gaps.len(),
                "groups": groups.len(),
            },
            "files": result.files.values().collect::<Vec<_>>(),
            "buckets": {
                "clean": { "count": clean_files.len(), "files": clean_files },
                "auto_fixed": { "count": auto_fixed_files.len(), "files": auto_fixed_files },
                "manual_fixes_needed": { "count": manual_fix_files.len(), "files": manual_fix_files },
                "operational_error": {
                    "count": result.operational_problems.len(),
                    "problems": result.operational_problems.values().collect::<Vec<_>>(),
                },
            },
            "clean_files": clean_files,
            "auto_fixed_files": auto_fixed_files,
            "manual_fix_files": manual_fix_files,
            "uncovered_files": result.uncovered_files,
            "not_applicable_files": result.not_applicable_files,
            "groups": groups,
            "reports": result.reports,
            "artifacts": result.artifacts.values().collect::<Vec<_>>(),
            "artifact_paths": artifact_paths,
            "artifact_contents": artifact_contents,
            "operational_problems": result.operational_problems,
            "coverage_gaps": result.coverage_gaps,
        }))
        .map_err(|error| ReportingError::Context(error.to_string()))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TemplateRun<'a> {
    pub id: &'a str,
    pub project_root: &'a Path,
    pub summary_path: &'a Path,
    pub state_directory: &'a Path,
}

fn files_with_status(result: &DeferredRunResult, status: FileStatus) -> Vec<&FileResult> {
    result
        .files
        .values()
        .filter(|file| file.status == status)
        .collect()
}

fn associated_artifact_paths(result: &DeferredRunResult, files: &[&FileResult]) -> Vec<PathBuf> {
    files
        .iter()
        .flat_map(|file| file.reports.iter())
        .flat_map(|report| report.artifact_ids.iter())
        .filter_map(|id| result.artifacts.get(id))
        .map(|artifact| artifact.absolute_path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn nonempty(message: String) -> Option<String> {
    (!message.trim().is_empty()).then_some(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactClassification, CommandPhase, FileAssessment, RunArtifact, ToolReport};

    fn artifact(id: &str, report_id: &str, path: &str, contents: &str) -> RunArtifact {
        RunArtifact {
            id: id.into(),
            absolute_path: PathBuf::from(path),
            run_relative_path: PathBuf::from(format!("tools/{id}.log")),
            media_type: "text/plain".into(),
            tool_id: Some("tool".into()),
            workflow_id: Some("lint".into()),
            job_id: Some("000".into()),
            report_id: Some(report_id.into()),
            phase: CommandPhase::FinalCheck,
            classification: ArtifactClassification::Issues,
            exit_code: Some(1),
            program: Some("tool".into()),
            arguments: vec!["check".into()],
            working_directory: Some(PathBuf::from("/repo")),
            files: vec![PathBuf::from("/repo/example.cpp")],
            candidate_files: vec![PathBuf::from("/repo/example.cpp")],
            changed_files: Vec::new(),
            contents: contents.into(),
        }
    }

    fn run<'a>() -> TemplateRun<'a> {
        TemplateRun {
            id: "run",
            project_root: Path::new("/repo"),
            summary_path: Path::new("/state/run/summary.json"),
            state_directory: Path::new("/state"),
        }
    }

    #[test]
    fn defaults_group_c_and_cpp_together_and_fallback_to_other() {
        let reporter = DeferredReporter::new(&pkl::DeferredReporting::default()).unwrap();
        let mut result = DeferredRunResult::default();
        for path in ["/repo/a.h", "/repo/b.cpp", "/repo/data.unknown"] {
            result.record_file(FileAssessment::new(path, FileStatus::Clean));
        }
        reporter.apply_groups(&mut result, Path::new("/repo"));
        assert_eq!(result.files[Path::new("/repo/a.h")].group_id, "c-cpp");
        assert_eq!(result.files[Path::new("/repo/b.cpp")].group_id, "c-cpp");
        assert_eq!(
            result.files[Path::new("/repo/data.unknown")].group_id,
            "other"
        );
    }

    #[test]
    fn first_matching_custom_group_wins() {
        let config = pkl::DeferredReporting {
            groups: vec![
                pkl::FileGroup {
                    id: "first".into(),
                    display_name: "First".into(),
                    include: vec!["**/*.rs".into()],
                },
                pkl::FileGroup {
                    id: "second".into(),
                    display_name: "Second".into(),
                    include: vec!["src/**".into()],
                },
            ],
            ..Default::default()
        };
        let reporter = DeferredReporter::new(&config).unwrap();
        let mut result = DeferredRunResult::default();
        result.record_file(FileAssessment::new("/repo/src/lib.rs", FileStatus::Clean));
        reporter.apply_groups(&mut result, Path::new("/repo"));
        assert_eq!(
            result.files[Path::new("/repo/src/lib.rs")].group_id,
            "first"
        );
    }

    #[test]
    fn defaults_render_buckets_pluralize_and_keep_clean_agent_empty() {
        let reporter = DeferredReporter::new(&pkl::DeferredReporting::default()).unwrap();
        let mut result = DeferredRunResult::default();
        result.record_file(FileAssessment::new("/repo/one.rs", FileStatus::Clean));
        result.record_file(FileAssessment::new("/repo/two.rs", FileStatus::AutoFixed));
        reporter.apply_groups(&mut result, Path::new("/repo"));
        let rendered = reporter.render(&result, run()).unwrap();
        assert!(rendered.buckets.clean.user.contains("1 clean file:"));
        assert!(rendered.buckets.clean.user.contains("one.rs"));
        assert!(rendered.buckets.clean.agent.is_empty());
        assert!(
            rendered
                .buckets
                .auto_fixed
                .user
                .contains("Auto-fixed 1 file:")
        );
        assert_eq!(
            rendered.agent.as_deref(),
            Some("Auto-fixed 1 file; re-read changed files before editing further.")
        );
    }

    #[test]
    fn manual_agent_groups_files_and_links_all_reports() {
        let reporter = DeferredReporter::new(&pkl::DeferredReporting::default()).unwrap();
        let mut result = DeferredRunResult::default();
        let path = PathBuf::from("/repo/example.cpp");
        for (report_id, artifact_id, artifact_path) in [
            ("report-a", "artifact-a", "/state/a.log"),
            ("report-b", "artifact-b", "/state/b.log"),
        ] {
            let report = ToolReport {
                id: report_id.into(),
                tool_id: report_id.into(),
                tool_name: report_id.into(),
                workflow_id: "lint".into(),
                job_id: "000".into(),
                candidate_files: vec![path.clone()],
                changed_files: Vec::new(),
                initial_check: Some(crate::CheckOutcome::Issues),
                fix_attempted: false,
                final_check: Some(crate::CheckOutcome::Issues),
                conservative_attribution: false,
                artifact_ids: vec![artifact_id.into()],
            };
            result.record_conservative_report(report, FileStatus::ManualFixesNeeded);
            result.record_artifact(artifact(artifact_id, report_id, artifact_path, report_id));
        }
        reporter.apply_groups(&mut result, Path::new("/repo"));
        let rendered = reporter.render(&result, run()).unwrap();
        assert!(
            rendered
                .buckets
                .manual_fixes_needed
                .user
                .contains("1 file needs manual fixes across 1 group: example.cpp")
        );
        let agent = rendered.agent.unwrap();
        assert!(
            agent.contains("C/C++: example.cpp"),
            "rendered agent message: {agent:?}"
        );
        assert!(agent.contains("/state/a.log"));
        assert!(agent.contains("/state/b.log"));
    }

    #[test]
    fn master_receives_raw_and_rendered_values_and_artifact_views_are_independent() {
        let mut config = pkl::DeferredReporting::default();
        config.clean.user = "sub={{ counts.clean }}".into();
        config.master_user = "{{ rendered_buckets.clean.user }}|{{ buckets.clean.count }}|{{ artifact_paths | length }}".into();
        config.master_agent = "{{ artifact_contents['/state/a.log'] }}".into();
        let reporter = DeferredReporter::new(&config).unwrap();
        let mut result = DeferredRunResult::default();
        result.record_file(FileAssessment::new("/repo/a.rs", FileStatus::Clean));
        result.record_artifact(artifact("a", "report", "/state/a.log", "artifact bytes"));
        reporter.apply_groups(&mut result, Path::new("/repo"));
        let rendered = reporter.render(&result, run()).unwrap();
        assert_eq!(rendered.user.as_deref(), Some("sub=1|1|1"));
        assert_eq!(rendered.agent.as_deref(), Some("artifact bytes"));
    }

    #[test]
    fn empty_template_suppresses_one_audience() {
        let mut config = pkl::DeferredReporting::default();
        config.manual_fixes_needed.user.clear();
        config.master_user = "{{ rendered_bucket_lists.user | join('') }}".into();
        let reporter = DeferredReporter::new(&config).unwrap();
        let mut result = DeferredRunResult::default();
        result.record_file(FileAssessment::new(
            "/repo/a.rs",
            FileStatus::ManualFixesNeeded,
        ));
        reporter.apply_groups(&mut result, Path::new("/repo"));
        let rendered = reporter.render(&result, run()).unwrap();
        assert!(rendered.user.is_none());
        assert!(rendered.agent.is_some());
    }
}
