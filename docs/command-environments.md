# Command-hook environments

Hook input is not always confined to JSON on stdin. Claude Code and Codex also
put harness state in a command hook's process environment.
`agent-hook-kit` parses that state into a harness-native type and passes it to
the handler explicitly:

```text
stdin bytes + declared environment variables
    -> native input + native command environment
    -> handler(input, environment, runtime_context)
    -> native command output
```

This page describes the selected 46-event inventory. The contract is scoped to
the **command handler binding**. HTTP handlers receive request data, not a hook
subprocess environment; Claude HTTP header interpolation is a separate,
allowlisted configuration feature. HTTP, prompt, agent, MCP, and other
non-command bindings are outside HookKit's implementation scope for this
iteration, not alternate runtime paths supplied elsewhere in the workspace.

## Contract boundary

`EnvironmentVariables` is a deterministic UTF-8 map of only the exact names and
dynamic prefixes declared by a `CommandEnvironmentSpec`. It is not a snapshot of
the complete child process environment.

A command may still inherit arbitrary parent-process variables, and Codex hook
configuration can add or override variables. Those ambient and
user-configured values are not harness identity or lifecycle state, so the
library does not place them in the typed contract. In particular:

- `NoCommandEnvironment` and `AntigravityCommandEnvironment` mean “no modeled
  harness-provided state,” not “the child process has an empty environment.”
- Environment values never select a harness or event. Selection remains an
  explicit type, `HarnessId`, or validated event discriminator.
- A matched variable with a non-Unicode value is rejected instead of being
  silently changed.
- `Debug` for `EnvironmentVariables` and Claude plugin options prints names but
  redacts values, because configured options can contain secrets.
- `CLAUDE_CODE_CHILD_SESSION=1` is a command-hook marker, not evidence that the
  hook is running in a subagent. Claude subagent identity comes from native JSON
  fields such as `agent_id` and `agent_type`.

## Exhaustive event/environment matrix

The rows below name every event in the selected snapshots: 31 Claude Code, 10
Codex, and 5 Antigravity events.

For every Claude Code row, the required baseline is `CLAUDECODE=1`,
`CLAUDE_CODE_CHILD_SESSION=1`, `CLAUDE_CODE_SESSION_ID`, and
`CLAUDE_PROJECT_DIR`. Every row also permits:

- `CLAUDE_EFFORT` and `TRACEPARENT` when supplied;
- local Remote Control state in `CLAUDE_CODE_BRIDGE_SESSION_ID`, or cloud state
  as `CLAUDE_CODE_REMOTE=true` together with `CLAUDE_CODE_REMOTE_SESSION_ID`;
- plugin state as the non-empty pair `CLAUDE_PLUGIN_ROOT` and
  `CLAUDE_PLUGIN_DATA`, plus zero or more `CLAUDE_PLUGIN_OPTION_<KEY>` values.

<!-- markdownlint-disable MD013 -->

| Harness and Rust type | Events | Event-specific typed environment |
| --- | --- | --- |
| Claude Code — `ClaudeCommandEnvironment` | `SessionStart`, `Setup`, `CwdChanged`, `FileChanged` | Claude baseline and conditionals above; `CLAUDE_ENV_FILE` is required and non-empty on these four events. |
| Claude Code — `ClaudeCommandEnvironment` | `InstructionsLoaded`, `UserPromptSubmit`, `UserPromptExpansion`, `MessageDisplay`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PostToolUseFailure`, `PostToolBatch`, `PermissionDenied`, `Notification`, `SubagentStart`, `SubagentStop`, `TaskCreated`, `TaskCompleted`, `Stop`, `StopFailure`, `TeammateIdle`, `ConfigChange`, `DirectoryAdded`, `WorktreeCreate`, `WorktreeRemove`, `PreCompact`, `PostCompact`, `SessionEnd`, `Elicitation`, `ElicitationResult` | Claude baseline and conditionals above. `CLAUDE_ENV_FILE` is ignored for these events. |
| Codex — `CodexCommandEnvironment` | `SessionStart`, `SubagentStart`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `UserPromptSubmit`, `SubagentStop`, `Stop` | Ordinary hooks have no modeled Codex variable. Plugin hooks provide `PLUGIN_ROOT`, `PLUGIN_DATA`, `CLAUDE_PLUGIN_ROOT`, and `CLAUDE_PLUGIN_DATA` together; the Claude-compatible aliases must equal the canonical paths. |
| Antigravity — `AntigravityCommandEnvironment` | `PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop` | No environment variable is defined by the official hook contract. Invocation state remains in stdin JSON. |

<!-- markdownlint-enable MD013 -->

`ClaudeCommandEnvironment` cross-checks `CLAUDE_CODE_SESSION_ID` against the
native input's `session_id`, and checks `CLAUDE_EFFORT` against `effort.level`
when both exist. Malformed variable groups and redundant-value mismatches fail
before the handler runs.

Several similarly named values are deliberate exclusions. Claude exposes the
active model in native JSON rather than a `CLAUDE_MODEL` hook variable. Codex's
`CODEX_THREAD_ID` belongs to shell-tool subprocesses, not the pinned hook
command runner. Antigravity's public hook reference does not promise an
`ANTIGRAVITY_CONVERSATION_ID` variable, so the typed contract continues to use
the JSON `conversationId` even if a particular binary exposes an incidental
environment value.

## Runtime API

The stdin/stdout runners capture and parse the appropriate variables
automatically. Exact typed handlers receive `(input, environment, context)`:

```rust
use hookkit_claude::protocol::{SessionStart, SessionStartOutput};

fn main() -> std::process::ExitCode {
    hookkit_runtime::run_event::<SessionStart, _>(
        |input, environment, context| {
            assert_eq!(
                environment.session_id.as_str(),
                input.session_id.as_str()
            );
            assert_eq!(context.event().name(), "SessionStart");
            Ok(SessionStartOutput::with_context(format!(
                "Project root: {}",
                environment.project_dir
            )))
        },
    )
}
```

In-memory execution accepts an injected map before the handler. This is the
preferred test shim because it does not mutate process-global state:

```rust
use hookkit_claude::protocol::{SessionStart, SessionStartOutput};
use hookkit_core::EnvironmentVariables;

fn execute_fixture(bytes: Vec<u8>) -> hookkit_core::Result<()> {
    let variables = EnvironmentVariables::from_pairs([
        ("CLAUDECODE", "1"),
        ("CLAUDE_CODE_CHILD_SESSION", "1"),
        ("CLAUDE_CODE_SESSION_ID", "session-1"),
        ("CLAUDE_PROJECT_DIR", "/workspace"),
        ("CLAUDE_ENV_FILE", "/tmp/claude-hook-env"),
    ]);

    let emission = hookkit_runtime::execute_typed::<SessionStart, _>(
        bytes,
        &variables,
        |_input, environment, _context| {
            assert_eq!(environment.project_dir.as_str(), "/workspace");
            Ok(SessionStartOutput::no_op())
        },
    )?;
    assert_eq!(emission.exit_code(), 0);
    Ok(())
}
```

Applications that need ambient capture outside a `run_*` adapter can call
`hookkit_runtime::environment::capture_command_environment::<E>()`, where `E`
implements `CommandEnvironmentSpec`. Most hook binaries should let the runner
perform that step.

Selected and aligned APIs retain the same second-argument position with a
different static type:

<!-- markdownlint-disable MD013 -->

| Execution API | Handler environment type |
| --- | --- |
| `run_event::<E>` / `execute_typed::<E>` | `&E::CommandEnvironment` |
| `run_harness::<H>` / `execute_harness::<H>` | `&H::CommandEnvironment` |
| `dispatch_builtin_harness` / `execute_builtin_harness` | `&BuiltinCommandEnvironment` |
| `run_aligned_event::<PostToolUse>` / `execute_aligned_event::<PostToolUse>` | `&PostToolUseCommandEnvironment` |

<!-- markdownlint-enable MD013 -->

## Compatibility notes

This is an intentional pre-1.0 API change:

- `EventSpec` and `HarnessSpec` implementors must provide the new
  `CommandEnvironment` associated type. Downstream events with no native state
  can use `hookkit_core::NoCommandEnvironment`.
- Command handler closures now take three arguments. The environment is inserted
  between the native input and `RuntimeContext`.
- In-memory `execute_typed`, `execute_harness`, `execute_builtin_harness`,
  `execute_aligned_event`, and `execute_post_tool_use` calls must pass
  `&EnvironmentVariables` before the handler.
- The `run_*` adapters need no new call-site argument; they capture only the
  selected environment spec's declared names and prefixes.

## Source basis

The environment model was checked against current official documentation and,
where the command-spawn details live only in implementation, pinned official
source:

- Claude Code: [hooks reference](https://code.claude.com/docs/en/hooks),
  [environment variables](https://code.claude.com/docs/en/env-vars),
  [plugin reference](https://code.claude.com/docs/en/plugins-reference), and
  [cloud-session links](https://code.claude.com/docs/en/claude-code-on-the-web#link-output-back-to-the-session).
- Codex: [hooks reference](https://learn.chatgpt.com/docs/hooks), pinned
  [`discovery.rs`](https://github.com/openai/codex/blob/9e552e9d15ba52bed7077d5357f3e18e330f8f38/codex-rs/hooks/src/engine/discovery.rs#L227-L235),
  and pinned
  [`command_runner.rs`](https://github.com/openai/codex/blob/9e552e9d15ba52bed7077d5357f3e18e330f8f38/codex-rs/hooks/src/engine/command_runner.rs#L171-L177).
- Antigravity: [current hooks reference](https://antigravity.google/docs/hooks)
  and the selected snapshot's
  [official Markdown source](https://antigravity.google/assets/docs/antigravity-2-0/hooks.md).

The event inventory remains governed by the immutable snapshots under
[`contracts/`](../contracts/README.md). Its separately versioned
[command-environment supplement](../contracts/supplements/command-environments/command-environments-2026-08-05-r2/)
is the machine-validated evidence behind the command-process state summarized
here.
