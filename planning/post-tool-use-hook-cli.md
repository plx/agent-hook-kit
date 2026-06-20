# Post-Tool-Use Hook CLI Inventory Plan
## Spec-Driven Wrappers For Existing Formatters, Linters, And Checkers

Version: draft 0.1

## 1. Purpose

The original motivation for `agent-hook-kit` is a high-quality inventory of
post-tool-use hooks that wrap existing developer tools. These hooks should make
agent edits safer and more ergonomic by:

1. detecting files touched by the agent;
2. running the right formatter, linter, checker, or autofixer;
3. determining whether the tool changed files and whether issues remain;
4. giving the user full diagnostics without polluting model context;
5. giving the agent concise, actionable feedback when the harness supports it.

The `ruff-agent-hook` prototype proved the value of this pattern, but hand-writing
one Rust CLI per external tool will not scale. This document lays out a route to
streamline creating many such tools while preserving correctness and escape
hatches.

The near-term plan is to develop these hook binaries inside this repository for
convenience. Once the design stabilizes, the inventory can move into a separate
repo while this library keeps the reusable runtime, common model, and runner
abstractions.

## 2. Goals

- Make common formatter/linter/checker hooks mostly data-driven.
- Avoid requiring diagnostic parsing for correctness.
- Preserve a precise distinction between:
  - no issues versus issues remain;
  - hook changed files versus hook left files unchanged;
  - tool-reported code issues versus operational tool failure.
- Keep tool unavailable and tool failure messages user-facing by default.
- Centralize harness differences in common output lowering.
- Support high-quality tool-specific guidance without per-tool Rust boilerplate.
- Keep enough escape hatches for tools with unusual command, workspace, or
  diagnostic behavior.

## 3. Non-Goals

- Do not try to model every external tool perfectly in the first pass.
- Do not make diagnostic parsers mandatory.
- Do not require Pkl or any external config evaluator in the first implementation.
- Do not assume every tool can be checked file-by-file.
- Do not treat "tool could not run" as equivalent to "source code has issues".

## 4. Core Design

The clean route is a hybrid of:

1. a Rust runner that owns execution, snapshots, classification, artifacts, and
   common output construction;
2. declarative tool specs that describe commands, file globs, scopes, exit-code
   semantics, and messages;
3. optional Rust adapters only for tools that do not fit the generic model.

In other words:

```text
CommonPostToolUseInput
  -> ToolSpec file selection
  -> snapshot write scope
  -> run format/fix/check phases
  -> snapshot write scope again
  -> classify outcome
  -> CommonPostToolUseOutput
  -> harness-aware lowering
```

Pkl is a good candidate for the declarative layer, especially because `hk` has
already shown this style can scale across a large builtin tool inventory. But the
first implementation should use an in-Rust `ToolSpec` struct and only add Pkl
once the schema has survived several real tools.

## 5. Outcome Model

The normal completed outcome should be a 2x2 matrix:

```text
                 unchanged          changed
clean            CleanUnchanged     CleanChanged
issues remain    IssuesUnchanged    IssuesChanged
```

This is better represented as two independent axes than as only four enum
variants:

```rust
pub enum IssueState {
    Clean,
    Issues,
}

pub enum ChangeState {
    Unchanged,
    Changed { files: Vec<PathBuf> },
}

pub struct CompletedToolOutcome {
    pub issues: IssueState,
    pub changes: ChangeState,
    pub diagnostics: Vec<DiagnosticReport>,
}
```

The 2x2 matrix covers tools that ran normally. Operational outcomes sit outside
the matrix:

```rust
pub enum ToolRunOutcome {
    Completed(CompletedToolOutcome),
    ToolUnavailable {
        executable: String,
        install_hint: Option<String>,
    },
    ToolFailed {
        phase: String,
        exit_code: Option<i32>,
        reason: FailureKind,
        diagnostics: DiagnosticReport,
    },
}
```

This separation matters. "Ruff reported lint issues" should drive code-fix
guidance. "Ruff is not installed" or "the tool crashed" should primarily be a
user-facing environment/tooling message.

## 6. Generic Classification Algorithm

Diagnostic parsing should not be required to determine the main outcome. The
generic classifier should use snapshots and exit-code semantics:

1. Find candidate modified files through `CommonPostToolUseInput::modified_files`.
2. Filter candidates using tool include/exclude globs.
3. Resolve the write scope for the tool.
4. Snapshot files in the write scope before running phases.
5. Run mutating phases such as format or fix.
6. Snapshot files in the write scope again.
7. Run a final read-only verify/check phase when available.
8. Classify:

```text
verify clean + no snapshot diff      -> Completed(Clean, Unchanged)
verify clean + snapshot diff         -> Completed(Clean, Changed)
verify issues + no snapshot diff     -> Completed(Issues, Unchanged)
verify issues + snapshot diff        -> Completed(Issues, Changed)
spawn error NotFound                 -> ToolUnavailable
configured failure exit/status       -> ToolFailed
ambiguous unexpected exit/status     -> ToolFailed by default
```

Snapshot comparison is the key fallback that avoids parsing tool output. It also
gives useful agent feedback: when the hook changed files, the agent should re-read
those files before continuing targeted edits.

## 7. Exit-Code Semantics

Tool specs should classify exit codes per phase. A phase that exits nonzero may
mean "issues remain" for one tool and "tool crashed" for another.

Example shape:

```rust
pub struct ExitCodePolicy {
    pub clean: Vec<i32>,
    pub issues: Vec<i32>,
    pub failure: Vec<i32>,
    pub unexpected: UnexpectedExitPolicy,
}

pub enum UnexpectedExitPolicy {
    Failure,
    Issues,
}
```

Example declarative shape:

```pkl
phases {
  ["verify"] {
    command = "ruff check {{files}}"
    cleanExitCodes = List(0)
    issuesExitCodes = List(1)
    failureExitCodes = List(2)
    unexpectedExitCodes = "failure"
  }
}
```

The default should be conservative:

- `0` means clean/success;
- unknown nonzero means `ToolFailed`, not source-code issues;
- a tool spec must explicitly identify nonzero "issues remain" codes.

## 8. Tool Phases

Most tools can be described as ordered phases:

```rust
pub struct ToolPhase {
    pub id: String,
    pub command: CommandTemplate,
    pub mode: PhaseMode,
    pub exit_codes: ExitCodePolicy,
    pub writes: WriteBehavior,
    pub diagnostics: DiagnosticCapture,
}

pub enum PhaseMode {
    Format,
    Fix,
    Verify,
    CheckOnly,
}

pub enum WriteBehavior {
    None,
    TargetFiles,
    MatchingGlobs,
    Workspace,
}
```

Typical tool shapes:

- Formatter: `format` phase writes target files; snapshot determines changed.
- Linter with autofix: `fix` phase writes target files, then `verify` determines
  clean versus issues.
- Check-only tool: no mutating phase; `verify` determines clean versus issues.
- Project-wide tool: phase runs at package/workspace scope and write scope may be
  broader than the exact agent-edited file.

## 9. Initial Rust Spec Shape

Before introducing Pkl, implement the runner against an ordinary Rust data
model. A rough starting point:

```rust
pub struct ToolSpec {
    pub id: String,
    pub display_name: String,
    pub executable: String,
    pub install_hint: Option<String>,
    pub file_selection: FileSelection,
    pub project_root: ProjectRootStrategy,
    pub phases: Vec<ToolPhase>,
    pub messages: ToolMessages,
}

pub struct FileSelection {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

pub struct ToolMessages {
    pub clean_changed_agent: Option<String>,
    pub issues_agent: Option<String>,
    pub issues_changed_agent: Option<String>,
    pub unavailable_user: Option<String>,
    pub failed_user: Option<String>,
}
```

The first `ruff-agent-hook` rewrite can become:

```rust
fn main() -> ExitCode {
    hookkit_tool_runner::run_spec(RUFF_SPEC)
}
```

or, if a small amount of customization is still needed:

```rust
fn main() -> ExitCode {
    hookkit_tool_runner::run_spec_with_adapter(RUFF_SPEC, RuffAdapter)
}
```

## 10. Declarative Spec Layer

After the Rust spec stabilizes, introduce a declarative format. Pkl is the best
candidate to evaluate first because:

- it can express typed configuration, defaults, validation, and inheritance;
- `hk` already uses Pkl for a large builtin inventory;
- some hk builtin metadata may be translated into our schema.

However, Pkl should define static tool behavior, not run the classification
itself. Rust should own process execution, snapshots, final classification,
artifact writing, and common output lowering.

Potential Pkl shape:

```pkl
name = "ruff"
displayName = "Ruff"
executable = "ruff"
installHint = "Install ruff with `uv tool install ruff` or add it to this project."

files {
  include = List("*.py", "**/*.py", "*.pyi", "**/*.pyi")
  exclude = List()
}

phases {
  ["format"] {
    command = "ruff format {{files}}"
    mode = "format"
    writes = "target-files"
    cleanExitCodes = List(0)
    failureExitCodes = List(2)
  }

  ["fix"] {
    command = "ruff check --fix --unfixable F401 {{files}}"
    mode = "fix"
    writes = "target-files"
    cleanExitCodes = List(0)
    issuesExitCodes = List(1)
    failureExitCodes = List(2)
  }

  ["verify"] {
    command = "ruff check {{files}}"
    mode = "verify"
    writes = "none"
    cleanExitCodes = List(0)
    issuesExitCodes = List(1)
    failureExitCodes = List(2)
  }
}

messages {
  cleanChangedAgent = "{{tool}} changed {{changed_files}}; re-read before editing further."
  issuesAgent = "{{tool}} reports issues; inspect diagnostics at {{diagnostics_path}}."
  issuesChangedAgent = "{{tool}} changed files and issues remain; re-read changed files, then inspect diagnostics at {{diagnostics_path}}."
}
```

The runner can eventually support baked specs using `include_str!` or generated
Rust constants, so a white-labeled binary does not need to load arbitrary config
at runtime unless we want it to.

## 11. Diagnostics

Diagnostics should be captured generically:

- command line;
- phase id;
- exit code or spawn error;
- stdout;
- stderr;
- changed files;
- verify result.

The default diagnostic artifact can be plain text. Tool-specific diagnostic
parsing can be optional and additive:

```rust
pub trait DiagnosticParser {
    fn parse(&self, logs: &[PhaseLog]) -> Option<StructuredDiagnostics>;
}
```

The parser should improve presentation, summaries, and file/line extraction. It
should not be required to determine `Clean` versus `Issues` or `Changed` versus
`Unchanged`.

## 12. Agent And User Messaging

Suggested default behavior:

| Outcome | User | Agent |
| --- | --- | --- |
| Clean + Unchanged | optional quiet/info | no feedback |
| Clean + Changed | concise notice | "files changed; re-read before continuing" |
| Issues + Unchanged | notice + diagnostics artifact | concise fix guidance |
| Issues + Changed | notice + diagnostics artifact | "files changed and issues remain" guidance |
| ToolUnavailable | user-facing install/config message | usually none |
| ToolFailed | user-facing error + diagnostics | only if agent can act on it |

The common lowering layer should keep enforcing harness capabilities. For example,
Codex `PostToolUse` may drop optional agent feedback under best-effort policy while
still sending user messages to stderr.

## 13. Repository Strategy

Initial development should stay in this repo:

- easier to evolve `hookkit-common`, `hookkit-runtime`, and runner APIs together;
- easier to keep examples and integration tests close to the library;
- lower process overhead while the schema and outcome model are still moving.

Expected local structure:

```text
crates/
  hookkit-tool-runner/        # new reusable runner crate
examples/
  ruff-agent-hook/
  prettier-agent-hook/
  eslint-agent-hook/
  cargo-agent-hook/
planning/
  post-tool-use-hook-cli.md
```

Once the runner API stabilizes:

- move the hook inventory into a new repo;
- keep `hookkit-tool-runner` either in this repo as a published crate or move it
  with the inventory if it remains too application-specific;
- keep this repo focused on core models, runtime plumbing, and harness behavior.

## 14. Candidate First Tools

Pick tools that stress different parts of the model:

1. `ruff`: existing prototype; format + fix + verify; Python file scope.
2. `prettier`: formatter; changed versus unchanged matters; usually no "issues"
   after write unless the command fails.
3. `eslint`: fix + verify; nonzero issue codes are normal.
4. `cargo fmt` / `cargo clippy --fix`: project/workspace scope and Rust-specific
   command constraints.
5. `biome`: combined formatter/linter behavior, useful for validating phase
   flexibility.

Avoid starting with the hardest project-wide tools only. The runner should first
prove the common file-scoped cases, then grow workspace behavior deliberately.

## 15. Implementation Phases

### Phase 1: Runner Core

- Add `hookkit-tool-runner`.
- Implement Rust `ToolSpec`.
- Implement file selection from `CommonPostToolUseInput::modified_files`.
- Implement phase execution and command templating.
- Implement snapshots and changed-file detection.
- Implement 2-axis completed outcome plus operational outcomes.
- Port `ruff-agent-hook` to the runner.

### Phase 2: More Builtins

- Add 2-4 more hook binaries in `examples/`.
- Keep specs in Rust constants.
- Add integration tests using fake tools with controlled exit codes and writes.
- Refine command templating, write scopes, verify scopes, and message defaults.

### Phase 3: Declarative Specs

- Define the Pkl-facing schema after the Rust schema stops changing weekly.
- Add a translator from Pkl or generated specs into `ToolSpec`.
- Evaluate whether hk builtin metadata can be translated into our model.
- Decide whether binaries load specs at runtime or bake a selected spec at compile
  time.

### Phase 4: Inventory Split

- Create a separate hook-inventory repo.
- Move stable hook binaries and specs.
- Keep cross-harness output behavior and semantic input helpers in
  `agent-hook-kit`.
- Publish or otherwise share the runner crate as needed.

## 16. Open Questions

- Should user notices for `Clean + Unchanged` be quiet by default?
- Should `Clean + Changed` always send agent feedback, or only when changed files
  intersect files the agent edited?
- How should snapshots handle generated files outside the selected path set?
- Should the runner snapshot by content hash, mtime/size, or both?
- How much workspace-root discovery belongs in the generic runner versus specs?
- Should missing executable be treated as a successful hook with user notice or a
  handler error? The current recommendation is successful hook with user-facing
  notice, because the hook itself worked.
- Should Pkl be a runtime dependency, a build-time generator, or only a source
  format translated into Rust constants?

## 17. Acceptance Criteria

The plan is working when:

- adding a straightforward formatter/linter wrapper does not require new Rust
  control flow;
- a hook can distinguish clean/issue and changed/unchanged without parsing
  diagnostics;
- missing tools and tool failures produce clear user-facing messages;
- agent feedback is precise about changed files and remaining issues;
- Codex limitations are handled by lowering policy, not by per-tool branching;
- integration tests can define fake tools and verify every outcome state.
