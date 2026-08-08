# Post-Tool-Use Runner Pkl Format Draft
## Annotated Review Version

Version: draft 0.2

> Implementation note (2026-07-21): the original document below remains the
> immediate PostToolUse format. Stop-time deferred execution now has an
> additive `ToolSpec.workflows` map. Each workflow contains a non-mutating
> `check`, an optional `remedy`, `checkScope`, and `invocation`; an optional
> `workflowOrder` provides stable override order. The immediate runner still
> executes `phases` exactly as documented below.
>
> When `workflows` is empty, the deferred runner compatibility-translates
> existing `phases`: each mutating phase is paired with the last enabled
> verifier and check-only tools become check-only workflows. Catalog validation
> rejects an unchecked remedy unless `unverifiedRemedyFallback` contains an
> explicit limitation; no enabled built-in uses that escape hatch. The complete
> checked inventory is in `builtin-deferred-workflow-audit.md`.
>
> `settings.deferredReporting` separately defines ordered file groups, four
> user/agent bucket template pairs, master user/agent templates, and optional
> empty-bucket rendering. Deferred Stop lowering applies `loweringPolicy` to
> the rendered audiences and records exact emitted/omitted/error disposition
> in `summary.json`; `ToolSpec.messages` remains immediate-runner-only.

The implemented deferred shape is:

```pkl
workflows {
  ["lint"] = new Workflow {
    check = new WorkflowCommand {
      argv = new Listing { "check"; new Files {} }
      exitCodes { issues = new Listing { 1 } }
    }
    remedy = new WorkflowCommand {
      argv = new Listing { "check"; "--fix"; new Files {} }
      writes = "target-files"
    }
    checkScope = "target-files" // or "workspace"
    invocation = "batch" // or "per-file" / "workspace"
  }
}
workflowOrder = new Listing { "lint" }
```

`WorkflowCommand.issuesOnStdout = true` adapts read-only commands such as
`gofmt -l` and `golines --dry-run`, which report dirty inputs on stdout while
retaining exit status zero. It only upgrades an otherwise-clean exit to source
issues. `ToolExecutable` is an argv token for the rare structured shell adapter
that must invoke the configured executable; the yq comparator uses it so an
executable override is preserved. Checks must declare `writes = "none"`, and
remedies must declare their actual write scope.

## 1. Purpose

This document defines a Pkl-shaped configuration format for one narrow binary:

```text
post-tool-use-agent-hook --claude|--codex|--gemini [--config PATH]
```

The binary is not a general-purpose hook framework. It implements exactly one
opinionated workflow:

1. parse a harness-native post-tool-use event;
2. extract files the agent likely modified;
3. select configured formatter/linter/checker tools for those files;
4. run each selected tool through a small phase pipeline such as format, fix,
   verify;
5. classify the result as clean/issues and changed/unchanged;
6. write detailed diagnostics for the user;
7. send concise agent feedback when the target harness can represent it;
8. lower the semantic result to Claude, Codex, or Gemini post-tool-use output.

The goal is to avoid compiling a separate binary for every wrapped tool. The
rote post-tool-use control flow lives in one binary; tool behavior is supplied by
Pkl builtins plus project/user overrides.

This file intentionally describes the product we want, not a generic hook system.

## 2. Non-Goals

- Do not support arbitrary hook events such as pre-tool-use, prompt-submit, stop,
  notifications, session start, or model-layer hooks.
- Do not expose a generic `hooks {}` registry like hk.
- Do not become a general task runner.
- Do not make Pkl execute arbitrary procedural hook logic.
- Do not require recompilation or a new Cargo target for each external tool.
- Do not parse shell commands deeply enough to infer every possible modified
  path. The runner uses the common post-tool-use semantic view and conservative
  path extraction.
- Do not make diagnostic parsing required for correctness. Exit-code policy and
  file snapshots drive the main outcome.

## 3. Current Rust Model Being Parameterized

The current implementation is Rust-first:

- `crates/hookkit-tool-runner/src/lib.rs` defines `ToolSpec`, `ToolPhase`,
  command argument templates, write behavior, exit-code policy, and messages.
- `hook-inventory/agent-hooks/src/specs.rs` builds concrete specs in Rust for
  `ruff`, `prettier`, `eslint`, `biome`, `cargo fmt`, and `cargo clippy`.
- small binaries such as `ruff-agent-hook` currently call
  `hookkit_tool_runner::run_spec(specs::ruff())`.

The desired next shape is:

```text
one post-tool-use runner binary
  + bundled Pkl builtin specs for many tools
  + project/user/local Pkl config that selects and customizes tools
```

The current Rust fields map cleanly to Pkl:

```rust
pub struct ToolSpec {
    pub id: String,
    pub display_name: String,
    pub executable: String,
    pub install_hint: Option<String>,
    pub file_selection: FileSelection,
    pub workspace_indicator: Option<String>,
    pub phases: Vec<ToolPhase>,
    pub messages: ToolMessages,
}
```

```rust
pub struct ToolPhase {
    pub id: String,
    pub mode: PhaseMode,
    pub program: Option<String>,
    pub args: Vec<CommandArgTemplate>,
    pub exit_codes: ExitCodePolicy,
    pub writes: WriteBehavior,
}
```

## 4. Runner Control Flow

The binary should have fixed control flow:

```text
stdin JSON
  -> parse selected harness
  -> require/ignore unless event is common PostToolUse
  -> load merged Pkl config
  -> get candidate modified files from CommonPostToolUseInput
  -> apply global and per-tool file filters
  -> build tool jobs, grouped by workspace indicator when configured
  -> snapshot write scopes
  -> run configured phases
  -> snapshot write scopes again
  -> classify each tool outcome
  -> write diagnostic artifacts when needed
  -> build CommonPostToolUseOutput
  -> lower to harness-native output
```

The user should not configure this control flow. They configure only the tool
catalog, selected tools, and policy knobs that are meaningful inside this one
workflow.

## 5. Pkl Design Decisions

### 5.1 Builtins are tool specs

Bundled Pkl files define reusable external tool specs. They describe local tool
commands and classification rules; they are not plugins.

Example builtin names:

- `Builtins.ruff`
- `Builtins.prettier`
- `Builtins.eslint`
- `Builtins.biome`
- `Builtins.cargoFmt`
- `Builtins.cargoClippy`

### 5.2 Project config selects a post-tool-use pipeline

There is no general `hooks` table. The top-level configuration is already for
post-tool-use behavior.

Project config should read like:

```pkl
tools = List(Builtins.ruff, Builtins.prettier, Builtins.eslint)
```

or, if stable ids are better for merging:

```pkl
tools {
  ["ruff"] = Builtins.ruff
  ["prettier"] = Builtins.prettier
  ["eslint"] = Builtins.eslint
}

run = List("ruff", "prettier", "eslint")
```

This draft uses the map-plus-run-list version because it supports clear
overrides and deterministic order.

### 5.3 Prefer argv templates over shell strings

Use structured argv tokens, not shell command strings. Rust owns process spawning,
quoting, and path expansion.

```pkl
argv = List("check", "--force-exclude", "--fix", new ExtraArgs {}, new Files {})
```

Shell strings can be added later as an explicit escape hatch if a real tool
requires it.

### 5.4 Runtime owns outcome classification

Pkl config declares exit-code semantics and write scopes. Rust still owns:

- spawning;
- snapshots;
- changed-file detection;
- diagnostics artifacts;
- message template rendering;
- common output construction;
- harness lowering.

For deferred turn completion, every executed initial check, remedy, and final
check is persisted separately in a unique session-state run bundle. Stable
run-relative paths encode deterministic tool/workflow/job/phase identity.
`summary.json` is the commit marker and includes typed artifact objects plus
separate artifact-path and artifact-content views for later templates. It is
written before pending-state disposition and therefore records that disposition
as planned rather than already acknowledged.

### 5.5 Configuration merging is the default

The binary should merge discovered config files by default, with project config
winning over user/global config. A later config must also be able to explicitly
reset prior state, either for the whole configuration or selected fields.

This keeps the normal path ergonomic while preserving an escape hatch for
projects that need a fully controlled policy.

### 5.6 Builtins are embedded Pkl source

Builtin tool specs should be tracked as Pkl files in the repo and embedded into
the binary. The binary evaluates those embedded specs rather than requiring a
network fetch or a separate installed builtin catalog.

### 5.7 No compatibility binaries

The first single-runner version should remove the per-tool binaries rather than
keep compatibility aliases. This avoids continuing to design around one binary
per tool.

## 6. Annotated Schema Sketch

This is review-oriented Pkl, not final checked `Config.pkl`.

```pkl
/// agent-hook-kit post-tool-use runner config, draft shape.
///
/// Project configs may eventually start with:
///
///   amends "package://.../agent-hook-kit-post-tool-use#/Config.pkl"
///   import "package://.../agent-hook-kit-post-tool-use#/Builtins.pkl"
///
/// The package/import strategy is still open. For early development, the binary
/// can embed builtins or load them from a repo-relative path.

/// Top-level settings for the fixed post-tool-use workflow.
class Settings {
  /// Number of independent tool jobs to run concurrently.
  ///
  /// 0 means auto-detect. The first implementation may still run serially until
  /// write-locking and workspace scopes are proven.
  jobs: UInt = 0

  /// Stop after an operational tool failure, such as a missing executable or a
  /// configured failure exit code.
  ///
  /// Source-code issues are not operational failures.
  failFast: Boolean = true

  /// Continue running later tools after an earlier tool reports source-code
  /// issues.
  continueAfterIssues: Boolean = true

  /// Global file exclusions applied before per-tool selection.
  exclude: List<String> = List(".git/**", "node_modules/**")

  /// How to handle optional common output intents unsupported by the target
  /// harness. Codex post-tool-use agent feedback is the main current example.
  loweringPolicy: "strict" | "best-effort" | "best-effort-with-warnings" =
    "best-effort-with-warnings"

  /// Default directory for diagnostic artifacts.
  ///
  /// Relative paths resolve from the project root discovered from config.
  diagnosticsDirectory: String? = ".agent-hook-kit/post-tool-use"

  /// What to do when a configured tool executable is missing.
  ///
  /// user-notice: hook succeeds and reports the missing tool to the user.
  /// hard-failure: hook fails so the harness treats the hook itself as failed.
  missingToolPolicy: "user-notice" | "hard-failure" = "user-notice"
}

/// Merge controls for multi-file config discovery.
///
/// Default behavior is additive/overriding merge. A config file can opt out of
/// earlier state before its own values are applied.
class Merge {
  /// Ignore all previously loaded config before applying this config.
  resetAll: Boolean = false

  /// Reset selected top-level fields before applying this config.
  reset: List<"settings" | "tools" | "run"> = List()

  /// Reset selected tool specs before applying this config.
  ///
  /// Useful when user config defines a tool and a project wants to replace it
  /// rather than amend it.
  resetTools: List<String> = List()
}

/// One external tool available to the fixed post-tool-use runner.
class ToolSpec {
  /// Stable id used in `tools`, `run`, diagnostics, skip lists, and template
  /// variables.
  id: String

  /// Human-readable name for user notices.
  displayName: String

  /// Default executable. Project/user/local config can override this.
  executable: String

  /// User-facing hint when the executable cannot be spawned.
  installHint: String?

  /// Candidate file selection for this tool.
  files: FileSelection = new FileSelection {}

  /// If set, split selected files into jobs by the nearest ancestor containing
  /// this indicator.
  ///
  /// Example: "Cargo.toml" for `cargo fmt` and `cargo clippy`.
  workspaceIndicator: String?

  /// Ordered phase map.
  ///
  /// Map keys are stable phase ids used for overrides. Pkl mappings preserve
  /// insertion order, but if this proves awkward we can add an explicit
  /// `phaseOrder: List<String>`.
  phases: Mapping<String, Phase> = new Mapping<String, Phase> {}

  /// Tool-specific message templates.
  messages: Messages = new Messages {}

  /// Tool-specific diagnostics override.
  diagnostics: Diagnostics = new Diagnostics {}

  /// Optional tool-level enable switch for project/local overrides.
  enabled: Boolean = true
}

class FileSelection {
  /// Include globs. Empty means all post-tool-use file candidates.
  include: List<String> = List()

  /// Exclude globs. Applied after global excludes and includes.
  exclude: List<String> = List()
}

class Phase {
  /// High-level purpose.
  ///
  /// format: mutating formatter
  /// fix: mutating autofixer
  /// verify: read-only final check
  /// check-only: read-only checker without a paired mutating phase
  mode: "format" | "fix" | "verify" | "check-only"

  /// Optional phase-specific executable. Defaults to ToolSpec.executable.
  program: String?

  /// Structured argv template.
  argv: List<String | ArgToken> = List()

  /// Exit-code classification.
  exitCodes: ExitCodes = new ExitCodes {}

  /// Which files this phase may write. Controls snapshot scope.
  writes: "none" | "target-files" | "matching-globs" | "workspace" = "none"

  /// Enable switch for project/local overrides.
  enabled: Boolean = true

  /// Phase-specific extra args inserted at `new ExtraArgs {}`.
  ///
  /// This avoids replacing the whole argv only to add one flag.
  extraArgs: List<String> = List()
}

/// Placeholder tokens for argv expansion.
abstract class ArgToken

/// Absolute paths of selected target files.
class Files extends ArgToken {}

/// Selected target files relative to the job workspace directory.
class WorkspaceFiles extends ArgToken {}

/// The job workspace directory.
class Workspace extends ArgToken {}

/// The discovered workspace indicator file, such as Cargo.toml.
class WorkspaceIndicator extends ArgToken {}

/// The project root discovered from config.
class ProjectRoot extends ArgToken {}

/// Phase-specific extra args.
class ExtraArgs extends ArgToken {}

class ExitCodes {
  /// Exit codes that mean the phase completed cleanly.
  clean: List<Int> = List(0)

  /// Exit codes that mean source-code issues remain.
  ///
  /// These are normal tool outcomes, not operational failures.
  issues: List<Int> = List()

  /// Exit codes that mean the tool failed operationally.
  failure: List<Int> = List()

  /// How to classify an unlisted exit code.
  unexpected: "failure" | "issues" = "failure"
}

class Diagnostics {
  /// Directory for this tool's diagnostic artifacts.
  ///
  /// If absent, Settings.diagnosticsDirectory is used.
  directory: String?
}

class Messages {
  /// Agent feedback when the tool changed files and final verify is clean.
  cleanChangedAgent: String =
    "{{ tool }} changed {{ changed_files | join(\", \") }}; re-read changed files before editing further."

  /// Agent feedback when issues remain and the tool did not change files.
  issuesAgent: String =
    "{{ tool }} reports issues; inspect diagnostics at {{ diagnostics_path }}."

  /// Agent feedback when the tool changed files and issues remain.
  issuesChangedAgent: String =
    "{{ tool }} changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}."

  /// User-facing missing-executable message. If absent, the runner builds a
  /// generic message using displayName, executable, phase, and installHint.
  unavailableUser: String?

  /// User-facing operational failure message. If absent, the runner builds a
  /// generic message using displayName, phase, and diagnostics path.
  failedUser: String?
}

settings: Settings = new Settings {}

/// Merge/reset directives for this config file.
///
/// This is meaningful only when the binary loads multiple config files. It is
/// ignored when a single explicit `--config` file is used.
merge: Merge = new Merge {}

/// Tool specs available to the runner.
///
/// Builtins and project config populate this map. Keys should match ToolSpec.id.
tools: Mapping<String, ToolSpec> = new Mapping<String, ToolSpec> {}

/// Ordered tool ids to run for every post-tool-use invocation.
///
/// This is the only "pipeline" this binary supports.
run: List<String> = List()
```

## 7. Builtin Example: Ruff

Equivalent to the current `specs::ruff()` Rust builder.

```pkl
ruff = new ToolSpec {
  id = "ruff"
  displayName = "Ruff"
  executable = "ruff"
  installHint = "install ruff with `brew install ruff` or add it to the project"

  files {
    include = List("*.py", "**/*.py", "*.pyi", "**/*.pyi")
  }

  phases {
    ["format"] = new Phase {
      mode = "format"
      argv = List("format", "--quiet", "--force-exclude", new ExtraArgs {}, new Files {})
      exitCodes {
        clean = List(0)
        failure = List(2)
      }
      writes = "target-files"
    }

    ["fix"] = new Phase {
      mode = "fix"
      argv = List("check", "--force-exclude", "--fix", new ExtraArgs {}, new Files {})
      exitCodes {
        clean = List(0)
        issues = List(1)
        failure = List(2)
      }
      writes = "target-files"
    }

    ["verify"] = new Phase {
      mode = "verify"
      argv = List("check", "--force-exclude", new ExtraArgs {}, new Files {})
      exitCodes {
        clean = List(0)
        issues = List(1)
        failure = List(2)
      }
    }
  }
}
```

## 8. Builtin Example: Cargo Clippy

Shows workspace grouping and matching-glob write scope.

```pkl
cargoClippy = new ToolSpec {
  id = "cargo-clippy"
  displayName = "cargo clippy"
  executable = "cargo"
  installHint = "install the Rust toolchain with clippy available"
  workspaceIndicator = "Cargo.toml"

  files {
    include = List("*.rs", "**/*.rs")
  }

  phases {
    ["fix"] = new Phase {
      mode = "fix"
      argv = List(
        "clippy",
        "--manifest-path",
        new WorkspaceIndicator {},
        "--fix",
        "--allow-dirty",
        "--allow-staged",
        "--quiet",
        new ExtraArgs {},
      )
      exitCodes {
        clean = List(0)
        issues = List(101)
        unexpected = "failure"
      }
      writes = "matching-globs"
    }

    ["verify"] = new Phase {
      mode = "verify"
      argv = List(
        "clippy",
        "--manifest-path",
        new WorkspaceIndicator {},
        "--quiet",
        new ExtraArgs {},
      )
      exitCodes {
        clean = List(0)
        issues = List(101)
        unexpected = "failure"
      }
    }
  }

  messages {
    issuesAgent =
      "cargo clippy reports issues; inspect diagnostics at {{ diagnostics_path }}."
    issuesChangedAgent =
      "cargo clippy changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}."
  }
}
```

## 9. Project Config Example

This is the intended normal project shape.

```pkl
amends "package://github.com/plx/agent-hook-kit/releases/download/v0.1.0/post-tool-use-agent-hook@0.1.0#/Config.pkl"
import "package://github.com/plx/agent-hook-kit/releases/download/v0.1.0/post-tool-use-agent-hook@0.1.0#/Builtins.pkl"

settings {
  exclude = List("node_modules/**", "dist/**", "vendor/**")
  loweringPolicy = "best-effort-with-warnings"
  diagnosticsDirectory = ".agent-hook-kit/post-tool-use"
  missingToolPolicy = "user-notice"
}

tools {
  ["ruff"] = (Builtins.ruff) {
    phases {
      ["fix"] {
        /// Disable unused-import removal during post-edit autofix while
        /// otherwise respecting ambient ruff config.
        extraArgs = List("--unfixable", "F401")
      }
    }

    messages {
      issuesAgent =
        "ruff: issues remain in {{ issue_files | join(\", \") }}; inspect {{ diagnostics_rel_path }}."
      issuesChangedAgent =
        "ruff changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect {{ diagnostics_rel_path }}."
    }
  }

  ["prettier"] = Builtins.prettier
  ["eslint"] = Builtins.eslint
}

run = List("ruff", "prettier", "eslint")
```

## 10. User/Local Override Example

This is for local machine preferences or temporary debugging.

```pkl
amends "./post-tool-use-agent-hook.pkl"

settings {
  jobs = 1
}

tools {
  ["ruff"] {
    executable = "uvx"

    phases {
      ["format"] {
        argv = List("ruff", "format", "--quiet", "--force-exclude", new ExtraArgs {}, new Files {})
      }

      ["fix"] {
        argv = List("ruff", "check", "--force-exclude", "--fix", new ExtraArgs {}, new Files {})
      }

      ["verify"] {
        enabled = false
      }
    }
  }
}
```

## 11. Merging And Reset Examples

Default merge:

```pkl
/// User config
tools {
  ["ruff"] = Builtins.ruff
}
run = List("ruff")
```

```pkl
/// Project config loaded later, merged over user config.
tools {
  ["prettier"] = Builtins.prettier
}
run = List("ruff", "prettier")
```

Project-level reset:

```pkl
/// Project wants a fully controlled pipeline, ignoring user/global tools.
merge {
  reset = List("tools", "run")
}

tools {
  ["ruff"] = Builtins.ruff
}

run = List("ruff")
```

Full reset:

```pkl
merge {
  resetAll = true
}

settings {
  failFast = true
  missingToolPolicy = "hard-failure"
}

tools {
  ["cargo-fmt"] = Builtins.cargoFmt
}

run = List("cargo-fmt")
```

## 12. Outcome Matrix

The runner classifies normal tool completion on two axes:

| Result | No files changed | Files changed |
| --- | --- | --- |
| Clean | quiet by default | user notice + concise agent reread feedback |
| Issues remain | user diagnostics + agent fix guidance | user diagnostics + agent reread-and-fix guidance |

Operational outcomes are outside this matrix:

- missing executable;
- spawn error;
- configured failure exit code;
- unexpected exit code classified as failure.

Missing tools are user-facing successful hook executions by default, but
`settings.missingToolPolicy = "hard-failure"` can make them fail the hook.
Other operational failures remain user-facing by default unless future policy
knobs prove necessary. Agent feedback should be reserved for cases where the
agent can act usefully, such as source issues in files it just modified.

## 13. Mapping From Current Rust Fields

| Rust field/type | Draft Pkl field/type | Notes |
| --- | --- | --- |
| `ToolSpec::id` | `ToolSpec.id` | Stable key in `tools`, `run`, diagnostics, templates. |
| `display_name` | `displayName` | User-visible and template `tool`. |
| `config_name` | removed | Per-tool binary compatibility; not central to one-runner design. |
| `executable` | `executable` | Tool-level default program. |
| `install_hint` | `installHint` | Missing-tool user message input. |
| `FileSelection` | `files` | Include/exclude globs. |
| `workspace_indicator` | `workspaceIndicator` | Splits jobs by nearest matching ancestor. |
| `ToolPhase::id` | `phases` map key | Stable override key. |
| `PhaseMode` | `Phase.mode` | Same four modes. |
| `ToolPhase::program` | `Phase.program` | Phase-specific executable. |
| `CommandArgTemplate` | `ArgToken` subclasses | Placeholder encoding still open. |
| `ExitCodePolicy` | `ExitCodes` | Same clean/issues/failure/unexpected model. |
| `WriteBehavior` | `Phase.writes` | Same snapshot scope model. |
| `ToolMessages` | `Messages` | MiniJinja templates rendered by Rust. |

## 14. Resolved Format Decisions

- Project config uses `tools + run`.
- Object-token argv syntax such as `new Files {}` is acceptable.
- `extraArgs` handles most local customization; full `argv` replacement remains
  the escape hatch.
- Phase maps preserve order sufficiently for v0; no `phaseOrder` field for now.
- Config files merge by default, with explicit reset support.
- Builtins are embedded Pkl tracked as source in the repo.
- Old per-tool binaries disappear immediately.
- `workspaceIndicator` is sufficient for v0.
- Missing tools are configurable into hard hook failures.
- `Clean + Unchanged` is quiet by default.

## 15. Remaining Review Questions

1. What exact config discovery order should the binary use for user/global,
   project, and local ignored files?
2. Should `merge.resetAll` be allowed in user/global config, or only project and
   explicit `--config` files?
3. Should hard missing-tool failures use harness blocking output when available,
   or should they fail the hook process uniformly?
4. Should embedded Pkl be evaluated at runtime, or compiled/generated into Rust
   structs at build time while keeping Pkl as the source of truth?

## 16. Acceptance Criteria

This format is good enough for v0 when:

- one binary can run `ruff`, `prettier`, `eslint`, `biome`, `cargo fmt`, and
  `cargo clippy` from Pkl specs without per-tool recompilation;
- project config can select an ordered set of tools;
- project config can disable a phase, change an executable, add phase args, and
  override messages;
- project config can explicitly reset merged user/global state;
- missing tools can be configured as either user notices or hard hook failures;
- the runner still owns all post-tool-use control flow and harness lowering;
- no config shape implies support for arbitrary hook events;
- Codex limitations are handled by common lowering policy, not by per-tool specs.
