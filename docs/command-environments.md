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

This page describes the selected 50-event inventory: 33 Claude Code, 12 Codex,
and 5 Antigravity events. The contract is scoped to the **command handler
binding**. HTTP handlers receive request data, not a hook subprocess
environment; Claude HTTP header interpolation is a separate, allowlisted
configuration feature. HTTP, prompt, agent, MCP, and other non-command bindings
are outside HookKit's implementation scope for this iteration, not alternate
runtime paths supplied elsewhere in the workspace.

## Contract boundary

`EnvironmentVariables` is a deterministic UTF-8 map of only the exact names and
dynamic prefixes declared by a `CommandEnvironmentSpec`. It is not a snapshot of
the complete child process environment.

A command still sees other variables. A Claude Code hook inherits Claude
Code's own environment, apart from the `OTEL_*` exporter variables and, when
`CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1`, the credentials Claude Code strips. A
Codex hook starts from an empty environment, receives a replay of the
environment Codex captured when the session's hook registry was created, and
then gets its plugin variables (see [Codex](#codex)). Those inherited values
are not harness identity or lifecycle state, so the library does not place them
in the typed contract. In particular:

- `NoCommandEnvironment` and `AntigravityCommandEnvironment` mean “no modeled
  harness-provided state,” not “the child process has an empty environment.”
- Environment values never select a harness or event. Selection remains an
  explicit type, `HarnessId`, or validated event discriminator.
- Because any modeled variable can also arrive as inherited ambient state,
  parsing fails only on state the harness itself would never produce for a
  hook. Empty or incomplete optional state is treated as absent; see the
  per-harness rules below.
- A declared exact name with a non-Unicode value is rejected instead of being
  silently changed. A variable matched only through a declared prefix, or named
  in the spec's `LENIENT_VARIABLE_NAMES` (Codex's plugin variables, which Codex
  always writes as UTF-8, and Claude Code's `CLAUDE_PID`), is skipped with a
  diagnostics warning instead, so stray inherited state cannot disable every
  hook.
- `Debug` for `EnvironmentVariables`, Claude plugin options, and the Claude
  messaging token prints names but redacts values, because they can contain
  secrets.
- `CLAUDE_CODE_CHILD_SESSION=1` is a command-hook marker, not evidence that the
  hook is running in a subagent. Claude subagent identity comes from native JSON
  fields such as `agent_id` and `agent_type`.

## Exhaustive event/environment matrix

The rows below name every event in the selected snapshots: 33 Claude Code, 12
Codex, and 5 Antigravity events.

### Claude Code

Every Claude Code row requires the baseline `CLAUDECODE=1`,
`CLAUDE_CODE_CHILD_SESSION=1` (Claude Code v2.1.172 or later),
`CLAUDE_CODE_SESSION_ID`, and `CLAUDE_PROJECT_DIR`. A missing, empty, or
different baseline value fails the hook. `CLAUDE_PROJECT_DIR` is the project
root where the session started; it does not follow `cd` in the Bash tool or
Claude's move into a worktree, while the input `cwd` does. HookKit therefore
uses it as the stable Claude Code root: aligned `project_roots()` and
`project_dir()` return it, and the bundled runners resolve a relative
`--state-dir` (and Stop-time configuration discovery) against it.

Every row also accepts this optional state:

- `CLAUDE_EFFORT` (Claude Code v2.1.133 or later) when the model supports
  effort, and `TRACEPARENT` (which may be empty) when trace context
  propagates. An empty `CLAUDE_EFFORT` is absent, and an unknown effort value
  is kept.
- `CLAUDE_PID`, Claude Code's own process ID (v2.1.214 or later), as a
  positive decimal integer. Claude Code never exports anything else, so an
  empty, signed, padded, non-digit, zero, overflowing, or non-UTF-8 value is
  inherited state and yields `claude_pid: None` instead of failing the hook.
- Execution location. A cloud session, Anthropic-hosted or, since v2.1.224, in
  a self-hosted environment, sets `CLAUDE_CODE_REMOTE=true` together with a
  non-empty `CLAUDE_CODE_REMOTE_SESSION_ID`; the marker without the ID fails
  the hook. Any other `CLAUDE_CODE_REMOTE` value, or an ID without the marker,
  is ambient state and yields a local session. The v2.1.224 floor comes from
  the Claude Code changelog, and the self-hosted configuration page documents
  `CLAUDE_CODE_REMOTE_SESSION_ID` in the environment the runner gives Claude
  Code, which hooks inherit. A local session may carry its Remote Control ID
  in `CLAUDE_CODE_BRIDGE_SESSION_ID` (v2.1.199 or later), which is ignored in
  a cloud session.
- Cross-session messaging, in sessions that bind an inbox socket, which is
  exported before any hook runs, including `SessionStart`. The supplement
  models two profiles: `CLAUDE_CODE_MESSAGING_SOCKET` (v2.1.224 or later), and
  the per-session secret `CLAUDE_CODE_MESSAGING_TOKEN` (v2.1.228 or later),
  which applies only alongside the socket. The socket may appear without the
  token; a token without a socket, or an empty value, is inherited state and
  is ignored.
- Plugin state as the non-empty pair `CLAUDE_PLUGIN_ROOT` and
  `CLAUDE_PLUGIN_DATA`, plus zero or more `CLAUDE_PLUGIN_OPTION_<KEY>` values.
  A complete pair with an empty half, or an option with an empty key, fails
  the hook. A lone half of the pair, or options without it, is ambient state
  from an unrelated parent process and is ignored.

<!-- markdownlint-disable MD013 -->

| Events | Event-specific typed environment |
| --- | --- |
| `SessionStart`, `Setup`, `CwdChanged`, `FileChanged` | Baseline and optional state above, plus `CLAUDE_ENV_FILE`. The hooks reference documents it for exactly these four events, but no sentence promises that it is set and the reference examples guard it with `[ -n "$CLAUDE_ENV_FILE" ]`, so it is optional: an absent or empty value yields `environment_file: None`. Exports from `SessionStart` and `Setup` persist for the session; exports from `CwdChanged` and `FileChanged` persist only until the next `CwdChanged` event, when Claude Code clears them. |
| `InstructionsLoaded`, `UserPromptSubmit`, `UserPromptExpansion`, `MessageDisplay`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PostToolUseFailure`, `PostToolBatch`, `PermissionDenied`, `Notification`, `SubagentStart`, `SubagentStop`, `TaskCreated`, `TaskCompleted`, `Stop`, `StopFailure`, `TeammateIdle`, `ConfigChange`, `DirectoryAdded`, `WorktreeCreate`, `WorktreeRemove`, `PreCompact`, `PostCompact`, `PreModelSwitch`, `PostModelSwitch`, `SessionEnd`, `Elicitation`, `ElicitationResult` | Baseline and optional state above. `CLAUDE_ENV_FILE` is ignored for these events. |

<!-- markdownlint-enable MD013 -->

`ClaudeCommandEnvironment` cross-checks `CLAUDE_CODE_SESSION_ID` against the
native input's `session_id`, and checks `CLAUDE_EFFORT` against `effort.level`
when both exist. Redundant-value mismatches fail before the handler runs.

The process working directory can differ from the input `cwd`: when the
session's directory no longer exists, Claude Code runs a command hook from the
first of the session's starting directory, the project root, the home
directory, or the system temporary directory that still exists.

### Codex

`CodexCommandEnvironment` covers all 12 Codex events: `SessionStart`,
`SessionEnd`, `SubagentStart`, `PreToolUse`, `PermissionRequest`,
`PostToolUse`, `PreCompact`, `PostCompact`, `UserPromptSubmit`,
`SubagentStop`, `Stop`, and `Interrupt`.

Ordinary hooks have no modeled Codex variable. Hooks discovered from a plugin,
on every event, receive `PLUGIN_ROOT`, `PLUGIN_DATA`, and the
Claude-compatible aliases `CLAUDE_PLUGIN_ROOT` and `CLAUDE_PLUGIN_DATA` together.
A complete set must have non-empty canonical paths and aliases equal to them;
an incomplete set is ambient state and yields no plugin. Since Codex 0.154.0
plugin hook sources are refreshed per turn, so the paths can change between
turns of one session.

Since Codex 0.149.0 a hook does not inherit Codex's live environment. Codex
starts it from an empty environment and replays the snapshot it took when the
session's hook registry was created, which reconfiguration keeps and does not
refresh, then applies the plugin variables. Codex hook configuration has no
`env` setting, so plugin discovery is the only source of hook-specific
variables. For every event, Codex removes five credential variables,
compared case-insensitively, from both sources:
`CODEX_EXEC_SERVER_NOISE_AUTH_TOKEN`, `NODE_REPL_AUTH_TOKEN`,
`OPENAI_FEDERATION_RULE_ID`, `OPENAI_IDENTITY_TOKEN_FILE`, and
`OPENAI_WORKLOAD_IDENTITY_CONTEXT`.

### Antigravity

`AntigravityCommandEnvironment` covers `PreToolUse`, `PostToolUse`,
`PreInvocation`, `PostInvocation`, and `Stop`. The official hook reference
defines no hook-process environment variable; invocation state remains in
stdin JSON. Variables named in the Antigravity CLI changelog, such as
`GEMINI_API_KEY` or `AGY_CLI_HIDE_LOGO`, configure the CLI itself and are not
hook state.

### Deliberate exclusions

Several similarly named values are deliberately not modeled. Claude exposes the
active model in native JSON rather than a `CLAUDE_MODEL` hook variable, and
`CLAUDE_JOB_DIR` and `CLAUDE_CODE_REMOTE_SESSION_UUID` are inherited session or
runner state, not hook variables. Codex sets `CODEX_THREAD_ID` and
`CODEX_SESSION_ID` for shell-tool and unified-exec processes, not hooks; a hook
sees them only when its Codex was itself launched from an outer Codex session,
and then they describe that outer session, so they must not be used as hook
identity. Antigravity's public hook reference does not promise an
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
implements `CommandEnvironmentSpec`, or its `_with_diagnostics` variant to
learn about skipped non-Unicode variables. Most hook binaries should let the
runner perform that step.

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
  [plugin manifest reference](https://code.claude.com/docs/en/plugins/manifest-reference),
  [plugin components](https://code.claude.com/docs/en/plugins/components),
  [cross-session messaging](https://code.claude.com/docs/en/cross-session-messaging#the-sessions-inbox-socket),
  [cloud-session links](https://code.claude.com/docs/en/cloud-environments#link-output-back-to-the-session),
  [self-hosted environments](https://code.claude.com/docs/en/self-hosted-environments),
  [self-hosted environment configuration](https://code.claude.com/docs/en/self-hosted-environments-configuration)
  (runner-provided `CLAUDE_CODE_REMOTE_SESSION_ID`), and the
  [Claude Code changelog](https://github.com/anthropics/claude-code/blob/2282079d6ac8824ec4b72a432a03e0c636e0512f/CHANGELOG.md)
  pinned by the selected snapshot (the v2.1.224 self-hosted and v2.1.133
  `CLAUDE_EFFORT` floors).
- Codex: [hooks reference](https://learn.chatgpt.com/docs/hooks) and pinned
  `rust-v0.159.2` source:
  [`discovery.rs`](https://github.com/openai/codex/blob/ff6aec96948b70d94983af2641a6b67c94faeff5/codex-rs/hooks/src/engine/discovery.rs#L262-L270)
  (plugin variables),
  [`registry.rs`](https://github.com/openai/codex/blob/ff6aec96948b70d94983af2641a6b67c94faeff5/codex-rs/hooks/src/registry.rs#L79)
  (environment snapshot),
  [`command_runner.rs`](https://github.com/openai/codex/blob/ff6aec96948b70d94983af2641a6b67c94faeff5/codex-rs/hooks/src/engine/command_runner.rs#L424-L435)
  (replay and scrubbing),
  [`shell_environment.rs`](https://github.com/openai/codex/blob/ff6aec96948b70d94983af2641a6b67c94faeff5/codex-rs/protocol/src/shell_environment.rs#L14-L20)
  (scrubbed names), and
  [`hook_config.rs`](https://github.com/openai/codex/blob/ff6aec96948b70d94983af2641a6b67c94faeff5/codex-rs/config/src/hook_config.rs#L165-L185)
  (no `env` setting).
- Antigravity: [current hooks reference](https://antigravity.google/docs/hooks)
  and its [official Markdown source](https://antigravity.google/docs/hooks.md).

The event inventory remains governed by the immutable snapshots under
[`contracts/`](../contracts/README.md). Its separately versioned
[command-environment supplement](../contracts/supplements/command-environments/command-environments-2026-09-30-r2/),
`command-environments-2026-09-30-r2`, is the machine-validated evidence behind
the command-process state summarized here. It succeeds
`command-environments-2026-09-30-r1` with the same profiles and event
mappings: it links the 2026-09-30 successor snapshots, and its changelog
evidence now cites the entries that add `CLAUDE_PROJECT_DIR` (1.0.58),
`CLAUDE_PLUGIN_DATA` (2.1.78), and `CLAUDE_EFFORT` (2.1.133), which the
earlier revision said did not exist.
