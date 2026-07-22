mod execution;
#[cfg(all(test, unix))]
mod execution_tests;
mod model;

pub(crate) use execution::{DeferredLog, ScheduledWorkflow, execute_deferred_workflows};

pub use model::{
    CheckOutcome, CommandPhase, CoverageGap, DeferredRunResult, FileAssessment, FileResult,
    FileStatus, OperationalProblem, RunArtifact, ToolReport, ToolReportRef,
};
