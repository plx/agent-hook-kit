mod execution;
#[cfg(all(test, unix))]
mod execution_tests;
mod lowering;
mod model;
mod reporting;

pub(crate) use execution::{
    DeferredLog, ScheduledWorkflow, WorkflowExecutionPolicy, execute_deferred_workflows,
};
pub(crate) use lowering::{StopLoweringMetadata, StopLoweringPlan, plan_stop_lowering_with};
pub(crate) use reporting::{
    BuiltinAudiences, DeferredReporter, RenderedBuckets, RenderedMessages, TemplateRun,
};

pub use model::{
    ArtifactClassification, CheckOutcome, CommandPhase, CoverageGap, DeferredRunResult,
    FileAssessment, FileResult, FileStatus, OperationalProblem, RunArtifact, ToolReport,
    ToolReportRef, UnavailableTool,
};
