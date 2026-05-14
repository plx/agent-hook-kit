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
- Pkl-driven post-tool-use runner with embedded tool catalog:
  - `hookkit-pkl-config` (Pkl evaluation, builtin specs, multi-file merge)
  - `hookkit-tool-runner` (ships the `post-tool-use-agent-hook` binary)
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
  hookkit-pkl-config/
  hookkit-tool-runner/
examples/
fixtures/
planning/
```

## Prerequisites

- Rust 2024 edition toolchain (`stable` works).
- [`pkl`](https://pkl-lang.org/main/current/pkl-cli/index.html) on `$PATH` —
  needed at runtime by the post-tool-use hook so it can evaluate embedded and
  user Pkl configs. Install with `brew install pkl` (macOS) or follow the
  upstream instructions for your platform.

## Build And Test

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

The `hookkit-pkl-config` and `hookkit-tool-runner` integration tests skip
themselves when `pkl` is not on `$PATH`, so the test suite still passes in
build environments without Pkl installed.

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

### Common `PostToolUse` Helpers

`hookkit-common` exposes a semantic post-tool view for formatter/linter-style
hooks. Hook logic can use normalized tool/result accessors and candidate
modified paths without inspecting harness JSON directly:

```rust
let view = post_tool.view_with_raw(&ctx.raw_input);
let paths = view.modified_files(ctx.cwd.as_std_path(), Some(project_root));
```

`CommonPostToolUseOutput` also supports semantic user notices, agent feedback,
diagnostics, and a lowering policy. Claude and Gemini receive agent feedback as
additional context. Codex currently cannot receive post-tool additional
context, so best-effort lowering drops that optional intent and can emit a
warning on `stderr` without contaminating hook JSON on `stdout`.

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

## `post-tool-use-agent-hook`

The reusable post-tool-use formatter/linter runner ships as a single binary
that selects tools and policy from Pkl configuration:

```bash
cat fixtures/claude/post_tool_use.json \
  | cargo run -q -p hookkit-tool-runner --bin post-tool-use-agent-hook -- --claude
```

The CLI accepts `--claude`, `--codex`, or `--gemini` to choose the harness,
and `--config PATH` to load a single Pkl file directly (bypassing discovery).

### Configuration discovery

When `--config` is not used the runner loads, in order:

1. `~/.agent-hook-kit/post-tool-use.pkl` (home/global)
2. each `<ancestor>/.agent-hook-kit/post-tool-use.pkl` walking up from `cwd`
   (root → leaf)
3. each `<ancestor>/.agent-hook-kit/post-tool-use.local.pkl` walking up from
   `cwd` (root → leaf, intended to be `.gitignore`d)

Later files override earlier ones. A file can opt out of earlier state with
`merge { resetAll = true }`, `merge { reset = new Listing { "tools"; "run" } }`,
or `merge { resetTools = new Listing { "ruff" } }`.

### Bundled builtins

`hookkit-pkl-config` embeds `Builtins.pkl` with reusable specs for:

- `Builtins.ruff` — Python format/fix/verify with ruff
- `Builtins.prettier` — JS/TS/CSS/HTML/JSON/Markdown formatter
- `Builtins.eslint` — JS/TS fix + verify
- `Builtins.biome` — JS/TS/JSON fix + verify
- `Builtins.cargoFmt` — Rust workspace formatter
- `Builtins.cargoClippy` — Rust workspace fix + verify

### Example project config

`.agent-hook-kit/post-tool-use.pkl`:

```pkl
amends "Config.pkl"
import "Builtins.pkl"

settings {
  diagnosticsDirectory = ".agent-hook-kit/post-tool-use"
  missingToolPolicy = "user-notice"
}

tools {
  ["ruff"] = (Builtins.ruff) {
    phases {
      ["fix"] {
        // keep unused-import removal out of the post-edit pass
        extraArgs = new Listing<String> { "--unfixable"; "F401" }
      }
    }
  }
  ["prettier"] = Builtins.prettier
}

run = new Listing<String> { "ruff"; "prettier" }
```

### Settings reference

| Field | Default | Purpose |
| --- | --- | --- |
| `settings.jobs` | `0` (auto) | Reserved for future parallelism. |
| `settings.failFast` | `true` | Stop after operational failures. |
| `settings.continueAfterIssues` | `true` | Keep running later tools after source issues. |
| `settings.exclude` | `[".git/**", "node_modules/**"]` | Global file exclusions applied before per-tool filters. |
| `settings.loweringPolicy` | `"best-effort-with-warnings"` | How to handle harness intents not natively expressible. |
| `settings.diagnosticsDirectory` | `".agent-hook-kit/post-tool-use"` | Where to write diagnostic artifacts. |
| `settings.missingToolPolicy` | `"user-notice"` | What to do when a configured tool executable is missing. Options: `"user-notice"`, `"hard-failure"`, `"harness-block"`. |

### Migration from per-tool binaries

Prior versions shipped six per-tool binaries (`ruff-agent-hook`,
`prettier-agent-hook`, etc.) configured by TOML. Those binaries have been
removed; replace them with the single `post-tool-use-agent-hook` binary and a
Pkl config that selects the same tool. For example:

```diff
- ruff-agent-hook --claude
+ post-tool-use-agent-hook --claude
```

…with `.agent-hook-kit/post-tool-use.pkl` referencing `Builtins.ruff` and any
overrides previously set in `.agent-hook-kit/ruff-agent-hook.toml`.

## `--dump-parsed` Debug Mode

All runtimes support parsed-event debug dumps to `stderr`:

```bash
cat fixtures/codex/pre_tool_use.json \
  | cargo run -q -p codex-bash-guard -- --dump-parsed
```

This prints a structured JSON line with harness, event, and parsed payload
without contaminating `stdout` hook output.

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
- `post-tool-use-agent-hook`:
  - loads merged Pkl config plus embedded builtin tool catalog,
  - selects modified files from the common post-tool semantic view,
  - runs each tool's phases for format/fix/verify commands,
  - classifies clean versus issues and changed versus unchanged from exit policies plus file snapshots,
  - reports missing tools and operational failures per `missingToolPolicy`,
  - writes remaining diagnostics to artifacts,
  - uses common output lowering so Codex limitations stay centralized.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Third-party crate licenses bundled with binary distributions are aggregated in
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md). That file is generated by
`cargo about` and validated in CI; regenerate it locally with
`scripts/regen-licenses.sh` after changing dependencies.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual-licensed as above, without any additional terms or conditions.
