# agent-hook-kit

Rust plumbing for agent hooks across Claude, Codex, and Gemini.

## What This Repository Provides

- Native input/output models per harness:
  - `hookkit-claude`
  - `hookkit-codex`
  - `hookkit-gemini`
- Cross-harness wrapper layer:
  - `hookkit-common`
- Runtime stdin/stdout/exit-code plumbing:
  - `hookkit-runtime`
- Shared core error/types:
  - `hookkit-core`
- Runnable examples:
  - `examples/claude-sessionstart-context`
  - `examples/codex-bash-guard`
  - `examples/gemini-beforetool-policy`
  - `examples/shared-posttool-autofix`

## Workspace Layout

```text
crates/
  hookkit-core/
  hookkit-runtime/
  hookkit-claude/
  hookkit-codex/
  hookkit-gemini/
  hookkit-common/
examples/
fixtures/
planning/
```

## Build And Test

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

## Quick Start: Native Runtime (`run_native`)

Minimal pattern:

```rust
use hookkit_core::Harness;
use hookkit_runtime::{NativeHookInput, NativeHookOutput, RuntimeContext};

fn main() -> std::process::ExitCode {
    hookkit_runtime::run_native(Harness::Claude, handle)
}

fn handle(
    input: NativeHookInput,
    _ctx: &RuntimeContext,
) -> hookkit_core::Result<NativeHookOutput> {
    match input {
        _ => Ok(NativeHookOutput::Claude(hookkit_claude::ClaudeHookOutput::Empty)),
    }
}
```

## Quick Start: Common Runtime (`run_common`)

Use when shared logic should run across harnesses:

```rust
use hookkit_common::input::CommonHookInput;
use hookkit_common::output::CommonHookOutput;
use hookkit_core::Harness;

fn main() -> std::process::ExitCode {
    hookkit_runtime::run_common(Harness::Claude, |input, _ctx| {
        match input {
            CommonHookInput::PostToolUse(_) => Ok(CommonHookOutput::empty()),
            _ => Ok(CommonHookOutput::empty()),
        }
    })
}
```

## Run The Examples With Fixtures

Claude session-start context:

```bash
cat fixtures/claude/session_start.json \
  | cargo run -q -p claude-sessionstart-context
```

Codex bash guard (JSON deny on blocked commands):

```bash
cat fixtures/codex/pre_tool_use.json \
  | cargo run -q -p codex-bash-guard
```

Gemini before-tool policy:

```bash
cat fixtures/gemini/before_tool.json \
  | cargo run -q -p gemini-beforetool-policy
```

Shared post-tool autofix (common runtime):

```bash
cat fixtures/claude/post_tool_use.json \
  | cargo run -q -p shared-posttool-autofix -- --claude
```

## `--dump-parsed` Debug Mode

All runtimes support parsed-event debug dumps to `stderr`:

```bash
cat fixtures/codex/pre_tool_use.json \
  | cargo run -q -p codex-bash-guard -- --dump-parsed
```

This prints a structured JSON line with harness, event, and parsed payload without contaminating `stdout` hook output.

## Example Behavior Summary

- `claude-sessionstart-context`:
  - injects additional model context on `SessionStart`.
- `codex-bash-guard`:
  - inspects `PreToolUse` Bash commands and emits deny JSON for blocked patterns.
- `gemini-beforetool-policy`:
  - denies or rewrites risky tool invocations in `BeforeTool`.
- `shared-posttool-autofix`:
  - runs a formatter/linter-autofix pipeline when applicable,
  - stays quiet on clean success,
  - prints concise user status to `stderr` when autofix/manual work occurs,
  - writes verbose manual diagnostics to a temp artifact and gives concise agent guidance when supported.
