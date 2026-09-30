//! Every implemented Claude Code event contract and its command output type,
//! at one stable path regardless of the module that defines it.

pub use crate::catalog::{
    ConfigChange, ConfigChangeOutput, CwdChanged, CwdChangedOutput, DirectoryAdded,
    DirectoryAddedOutput, Elicitation, ElicitationOutput, ElicitationResult,
    ElicitationResultOutput, FileChanged, FileChangedOutput, InstructionsLoaded,
    InstructionsLoadedOutput, MessageDisplay, MessageDisplayOutput, Notification,
    NotificationOutput, PermissionDenied, PermissionDeniedOutput, PermissionRequest,
    PermissionRequestOutput, PostCompact, PostCompactOutput, PostToolBatch, PostToolBatchOutput,
    PostToolUseFailure, PostToolUseFailureOutput, PreCompact, PreCompactOutput, PreToolUse,
    PreToolUseOutput, SessionEnd, SessionEndOutput, Setup, SetupOutput, Stop, StopFailure,
    StopFailureOutput, StopOutput, SubagentStart, SubagentStartOutput, SubagentStop,
    SubagentStopOutput, TaskCompleted, TaskCompletedOutput, TaskCreated, TaskCreatedOutput,
    TeammateIdle, TeammateIdleOutput, UserPromptExpansion, UserPromptExpansionOutput,
    UserPromptSubmit, UserPromptSubmitOutput, WorktreeRemove, WorktreeRemoveOutput,
};
pub use crate::model_switch::{
    PostModelSwitch, PostModelSwitchOutput, PreModelSwitch, PreModelSwitchOutput,
};
pub use crate::protocol::{
    PostToolUse, PostToolUseOutput, SessionStart, SessionStartOutput, WorktreeCreate,
    WorktreeCreateOutput,
};
