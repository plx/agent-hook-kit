mod execution;
#[cfg(all(test, unix))]
mod execution_tests;
mod lowering;
mod model;
mod reporting;

pub(crate) use execution::{DeferredLog, ScheduledWorkflow, execute_deferred_workflows};
pub(crate) use lowering::{StopLoweringMetadata, plan_stop_lowering};
pub(crate) use reporting::{DeferredReporter, RenderedBuckets, RenderedMessages, TemplateRun};

pub use model::{
    ArtifactClassification, CheckOutcome, CommandPhase, CoverageGap, DeferredRunResult,
    FileAssessment, FileResult, FileStatus, OperationalProblem, RunArtifact, ToolReport,
    ToolReportRef,
};
