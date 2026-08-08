# Copier-based Hook Project Templates

Status: implemented and locally path-validated; public Git preview and
crates.io publication gates remain

Date: 2026-08-07

Target: `agent-hook-kit` after `5ae45c2`

Implementation completed on 2026-08-07. The repository now contains the
task-free Copier template, canonical 47-event and 11-alignment catalogs,
catalog synchronization/checking, both output topologies, all state capability
modules and starter archetypes, package-namespaced CI, and a pinned Copier
acceptance matrix. The matrix covers maximal native projects, every
cross-harness subset, archetypes/state fidelity, crate coexistence, validation
failures, and a synthetic versioned update that preserves user-owned handlers.

The remaining gates require external release state rather than more local
template structure: publish a compatibility commit containing the generated
APIs, update the catalog to that reachable SHA, run a clean Git-source smoke,
and tag the preview. Switching the default to crates.io remains blocked on the
12-crate compatibility train and a clean locked crates.io render.

## 1. Recommendation

Build one independently updateable Copier template with two output modes:

1. **Full project** — create a new Rust workspace containing one hook CLI crate.
2. **CLI crate** — add one namespaced hook CLI crate, and optionally one
   namespaced GitHub Actions workflow, to an existing repository.

Both modes should produce the same application structure and use the same event
catalog, archetypes, state-capability modules, tests, and dependency renderer.
They differ only in which repository-level files the template owns.

The generated application should be one branded binary with explicit event
subcommands. A cross-harness binary also requires an explicit harness argument;
a single-harness binary fixes the harness at compile time. For example:

```text
acme-hooks --harness claude pre-tool
acme-hooks --harness codex post-tool
acme-hooks --harness antigravity turn-completion
```

and, for a single-harness project:

```text
acme-hooks pre-tool
acme-hooks session-start
```

The template should be task-free in its first version. `copier copy` and
`copier update` must not require `--trust`; formatting, compilation, and tests
are explicit generated setup commands and CI steps.

This proposal replaces the generated-application portions of
[`hookkit-clap-template-feasibility.md`](hookkit-clap-template-feasibility.md).
That note's conclusion about static CLI composition remains useful, but a new
runtime registry or `hookkit-templates` library is not required for the Copier
work.

## 2. Current facts that constrain the template

### 2.1 There are exactly three built-in harnesses

The current implementation registry contains 47 command events:

| Stable ID | Display name | Command events | Current snapshot |
| --- | --- | ---: | --- |
| `claude-code` | Claude Code | 31 | `docs-2026-08-05-r1` |
| `codex` | Codex CLI | 11 | `commit-1e59dc5-r1` |
| `antigravity` | Antigravity | 5 | `docs-2026-08-04-r1` |

The counts are explanatory snapshots, not a second hard-coded catalog. Template
coverage must be derived from
`contracts/status/implementation/registry.json`, including Claude's new
`DirectoryAdded` event and Codex's new `SessionEnd` event.

All three are normal built-in choices, and cross-harness mode defaults to all
three. There is no legacy fourth-harness compatibility mode, hidden choice, or
feature to preserve. Capability and fidelity labels are computed from the
selected set rather than assigning a permanent “primary” or “limited” tier to a
harness.

### 2.2 “Cross-harness hook” means a library alignment, not a same-looking name

Three aligned families are universal across all supported harnesses:

| Stable template ID | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| `pre_tool` | `PreToolUse` | `PreToolUse` | `PreToolUse` |
| `post_tool` | `PostToolUse` | `PostToolUse` | `PostToolUse` |
| `turn_completion` | `Stop` | `Stop` | `Stop` |

These are the universal choices available to every two- or three-harness
selection. The template does not infer portability by intersecting event name
strings; it reads explicit alignment records backed by sealed runtime markers.

The generated code must retain native arms. Pre-tool has portable
`allow`/`deny` helpers; post-tool and turn-completion need explicit native
lowering for each selected harness. Antigravity `PostToolUse` now carries a
typed originating tool name and arguments, so tool analysis works. It still
lacks a tool result or changed-file list, and its output is an empty JSON object
with no user- or agent-message channel.

### 2.3 Claude and Codex now form a richer selected-set tier

Every current Codex command event has a Claude event with the same lifecycle
meaning. In addition to the universal three, eight credible Claude/Codex
alignments are now available for deliberate library implementation:

<!-- markdownlint-disable MD013 -->

| Stable template ID | Portable floor | Native behavior kept outside the floor |
| --- | --- | --- |
| `permission_request` | allow or deny | Claude input/permission rewrites and interrupt controls |
| `pre_compact` | observe/no-op | Claude's compaction-specific block and Codex's broader stop control are not equivalent |
| `post_compact` | observe and optional system notice | Claude carries a summary; Codex does not |
| `session_start` | observe and add agent context | harness-specific source, title, watch-path, and skill fields |
| `session_end` | observe/no-op | Codex ignores output; Claude's output controls stay native-only |
| `subagent_start` | observe and add context | richer native message/control fields |
| `subagent_stop` | observe or block with a reason | native context and message-audience differences |
| `user_prompt_submit` | add context or block with a reason | richer native top-level controls |

<!-- markdownlint-enable MD013 -->

These eight pair-only families now have lossless `hookkit-common` wrappers,
sealed `hookkit-runtime` markers, exact native lowerings, and conformance tests.
Copier exposes a family only when its runtime marker exists and its declared
supported-harness set contains every selected harness. Consequently the
Claude/Codex pair sees all 11 families, while every Antigravity-containing set
sees only the compatible universal intersection.

The same selected-set rule applies within a family. Claude and Codex both
support `PreToolUse` input rewriting, so a cross-harness rewrite archetype can
be offered for that pair with explicit native lowering. Adding Antigravity
removes that output capability even though `pre_tool` itself remains aligned.

### 2.4 Only command hooks belong in the questionnaire

The contract ledger also records HTTP, prompt, agent, and MCP bindings. They are
not runtime implementation targets in this release and must not appear as
scaffoldable hook choices. The event catalog for the template is derived from
implemented `command` bindings only.

### 2.5 Harness selection does not yet minimize the dependency graph

`hookkit-runtime` and `hookkit-common` currently depend on all three native
adapters, and `hookkit-tool-access` enables all adapters in `hookkit-shell`.
Selecting one harness still matters for generated API shape, CLI behavior, and
available event choices, but does not yet produce a single-adapter transitive
build.

Feature-gating those library crates is a worthwhile follow-up, not a blocker for
the first template. Complete graph minimization will also need feature forwarding
through `hookkit-tool-access`, `hookkit-file-activity`, and
`hookkit-tool-runner`.

### 2.6 File activity is a coordinated suite

“Track probably touched files” is not a standalone boolean. Precise deferred
work needs three coordinated bindings that share one state root:

```text
SessionStart -> exact metadata observer on Claude/Codex
PostToolUse  -> direct file-activity producer on all three
Stop         -> deferred consumer on all three
```

Antigravity's originating call and arguments now support structured and shell
evidence at `PostToolUse`. It still has no exact `SessionStart`; its first lower
bound must be inferred from `PreInvocation(invocation_num == 0)` or the first
observed hook, so initial mtime reconciliation remains best effort. That is a
session-boundary fidelity difference, not a reason to exclude Antigravity from
file activity. The old `ModifiedFiles` projection should not be offered for new
projects; `hookkit-file-activity` is the supported path.

The runner and file-activity family names are currently fixed. Each generated
tool must therefore use a package-specific default state root so two generated
deferred suites cannot consume each other's pending window.

## 3. Goals and non-goals

### Goals

- Generate a compiling Rust 2024 CLI with a small, understandable module tree.
- Support every implemented native command event in single-harness mode.
- Offer only real library alignments in cross-harness mode.
- Compute cross-harness events and output capabilities from the selected
  harness set, including the richer Claude/Codex pair after its runtime support
  lands.
- Make high-level archetypes fast while retaining a fully custom route.
- Compose state capabilities without forcing every project to depend on all
  state crates.
- Generate fixtures and tests that exercise exact protocol output.
- Support both a new workspace and a crate added to an existing repository.
- Generate an optional, uniquely named GitHub Actions workflow.
- Remain updateable with Copier without claiming ownership of handler logic.
- Use a pinned Git dependency until the complete crate graph is public, then
  switch the release default to crates.io.

### Non-goals for the first version

- Editing a user's Claude, Codex, or Antigravity configuration in place.
- Detecting a harness from input JSON.
- Generating HTTP, prompt, agent, or MCP handlers.
- Mutating an existing root `Cargo.toml` automatically.
- Providing a dynamic Rust plugin or runtime command registry.
- Providing release packaging, installers, Homebrew formulae, or binary release
  workflows.
- Hiding native output-capability differences behind a lossy universal output.

The generated README can include registration command lines and configuration
fragments, but installation into live harness configuration remains an explicit
user step.

## 4. Copier structure

Keep one logical template because all components share the HookKit compatibility
version and update lifecycle. Copier can include configuration and Jinja
partials, but it does not declaratively execute and update a graph of child
templates. Splitting base, hooks, state, and CI into separately applied
templates would make ownership and upgrades harder without adding useful
independence.

Recommended source layout:

```text
copier.yml
templates/hook-project/
  questions/
    identity.yml
    topology.yml
    starter.yml
    harness.yml
    state.yml
    hooks.yml
    dependency.yml
    ci.yml
  catalog/
    event-scaffolds.yml
    alignment-families.yml
    archetypes.yml
    compatibility.yml
  template/
    ... rendered project tree ...
  tests/
    cases/
    snapshots/
```

The root `copier.yml` should:

- set `_min_copier_version: "9.17.1"`;
- set `_subdirectory` to `templates/hook-project/template`;
- assemble the question files with YAML `!include`;
- use `.jinja` as the template suffix;
- contain no `_tasks` in v1;
- reserve `_migrations` for later answer-key or path transitions that a normal
  smart update cannot express.

This makes the repository itself the template source during the initial
implementation. The template and HookKit release should use the same immutable
tag so the compatibility catalog can name the exact API it renders against:

```text
copier copy --vcs-ref v0.1.0 \
  https://github.com/prb/agent-hook-kit.git ./acme-hooks
```

For crate mode, run the same source with the existing repository root as the
destination and select `output_mode=crate` in the questionnaire. If template
changes eventually need a release cadence independent from the Rust crates,
move this one logical template to its own repository; do not introduce a
second competing tag stream in the current repository.

Pin the initial implementation and its render CI to Copier 9.17.1, the audited
minimum with the required Jinja sandbox/security fixes. Its documented features
cover typed and dynamic choices, multiselects, conditional paths, includes,
smart updates, migrations, external data, and distinct answer files for
repeated templates:

- <https://copier.readthedocs.io/en/stable/configuring/>
- <https://copier.readthedocs.io/en/stable/updating/>
- <https://copier.readthedocs.io/en/stable/changelog/#v9171-2026-08-04>

Conditional modules should use conditional path names, leaving `.jinja` outside
the condition. Coarse components remain separate directories/files rather than
one heavily branched source file.

## 5. Questionnaire

### 5.1 Ordering

Ask questions in this order:

1. output topology and identity;
2. starter/archetype;
3. harness mode and harness selection;
4. state capabilities;
5. hook selection;
6. dependency source;
7. CI;
8. optional archetype-specific configuration.

State precedes hook selection so a compound capability can make its required
hooks visible and selected by default. Important concrete selections should not
be hidden with `when`: Copier makes a skipped question's default available to
rendering but does not record that answer. Preset-derived harnesses, hooks, and
state capabilities should remain visible with good defaults so the answer file
contains the actual choices and future template updates are stable.

Use `when` only for genuinely irrelevant secondary fields, such as a Git
revision when `dependency_source != "git"`.

### 5.2 Proposed questions

<!-- markdownlint-disable MD013 -->

| ID | Type | Default | Notes |
| --- | --- | --- | --- |
| `output_mode` | choice | `project` | `project` or `crate` |
| `project_name` | string | — | Human-readable name |
| `package_name` | string | normalized project name | Kebab-case Cargo package; stable namespace |
| `binary_name` | string | `package_name` | Kebab-case installed command |
| `crate_path` | string | `crates/<package_name>` | Used in both modes; relative, no `..` |
| `description` | string | starter-derived | Cargo/README description |
| `license` | choice | `MIT OR Apache-2.0` | Explicit crate metadata in both output modes |
| `starter` | choice | `custom` | Custom or one initial archetype |
| `harness_mode` | choice | starter-derived | `cross` or `single` |
| `harnesses` | multiselect | all supported harnesses | Cross mode; at least two |
| `harness` | choice | starter-derived | Single mode |
| `state_capabilities` | multiselect | starter-derived | Concrete state modules |
| `aligned_hooks` | multiselect | starter-derived | Cross mode; dynamic valid families |
| `native_hooks` | multiselect | starter-derived | Single mode; dynamic per harness |
| `lowering_policy` | choice | `best-effort-with-warnings` | Runner/message archetypes; strict or either best-effort mode |
| `dependency_source` | choice | `git` before publication | `git`, `crates-io`, or `path` |
| `hookkit_git_rev` | string | compatibility catalog value | Required exact revision in Git mode |
| `hookkit_path` | string | — | HookKit repository root; development/local mode only |
| `github_actions` | bool | `true` in project mode | Optional namespaced workflow |

<!-- markdownlint-enable MD013 -->

Validation rules:

- names are normalized and path-safe; family names stay within
  `[A-Za-z0-9._-]`;
- `crate_path` is relative and contains neither an empty component nor `..`;
- single mode selects exactly one harness; cross mode selects at least two;
- at least one effective hook is selected;
- cross mode contains aligned-family IDs only, and each family has an
  implemented sealed runtime marker whose supported set contains every
  selected harness;
- a state capability's required hooks cannot be deselected;
- `file_activity` works for all three, but a selection that requires an exact
  session-start lower bound excludes Antigravity;
- an archetype must remain compatible with the chosen native output features;
- strict message requirements are rejected when a selected harness cannot
  represent the requested audience; they are never silently downgraded;
- Git mode requires a full immutable commit revision, never a branch name;
- path mode is clearly marked non-portable and is not the public default; its
  repository root must contain every selected `crates/hookkit-*` package.

`package_name` and `crate_path` are instance identity after the first copy.
Changing either also changes owned paths and, in crate mode, the answers-file
location. Treat a rename as an explicit migration/recopy operation rather than
an ordinary questionnaire update. `binary_name` is technically renameable but
also changes registration commands, CI names, and the default state namespace,
so it needs the same deliberate treatment.

### 5.3 Stable values and changing labels

Persist stable machine IDs such as `antigravity`, `turn_completion`, and
`record_queue`. Labels should be descriptive and may evolve, for example:

```text
Turn completion (Claude / Codex / Antigravity Stop)
Post-tool analysis (all three; Antigravity has call arguments but no result or messages)
Session start (Claude + Codex; exact session boundary)
```

Changing a display label must not rewrite an answers file or select a different
event.

### 5.4 Resolved manifest

Render a small `hookkit-template.manifest.yml` beside the generated crate. It is
human-readable, not a replacement for Copier's answers file, and records:

- template version;
- output mode;
- starter and whether its defaults were customized;
- requested and effective hooks;
- harness set;
- aligned-family availability and the selected portable output floor;
- per-harness fidelity facts, including exact versus inferred session start,
  tool-result availability, and message audiences;
- state capabilities;
- lowering policy;
- HookKit dependency source/version/revision;
- generated CI filename;
- package-specific state family/root identifiers.

This makes automatic additions, such as file activity's support commands,
reviewable in the generated diff.

## 6. Selection and dependency resolution

Treat the answer set as declarative input to a small deterministic resolver:

```text
available aligned families
  = implemented alignment records
  where selected harnesses ⊆ family supported harnesses

available output capabilities
  = intersection of native capabilities across selected harnesses
  constrained by the family's declared portable floor

effective hooks
  = explicitly selected hooks
  + hooks required by selected state capabilities
  + support hooks required by the selected archetype

effective crates
  = runtime/core/native crates required by effective hooks
  + crates required by selected state capabilities
  + crates required by archetype support modules
```

The resolver is template data and Jinja logic, not a post-copy executable. It
must render the same result in interactive, defaults-file, recopy, and update
modes. Alignment records explicitly name their sealed runtime marker and
supported harnesses. The resolver never manufactures a record from matching
wire names.

Always render only direct dependencies used by generated Rust source. Current
transitive behavior may compile all adapters anyway, but the generated manifest
should state intent so future library feature-gating can immediately take
advantage of the selection.

In path mode, `hookkit_path` is the HookKit repository root. The dependency
macro appends each direct package path, such as
`<hookkit_path>/crates/hookkit-runtime`; it must not reuse the repository-root
literal as though every crate lived there.

Typical dependency additions are:

<!-- markdownlint-disable MD013 -->

| Selection | Direct generated dependencies |
| --- | --- |
| Native/aligned handler | `hookkit-core`, `hookkit-runtime`, `clap` |
| Single native harness | selected `hookkit-<harness>` crate |
| Cross-harness hook | `hookkit-common` plus native crates referenced by lowering |
| Tool access | `hookkit-tool-access`; direct `hookkit-shell` only for custom profiles/rules |
| Session state | `hookkit-session-state`, `serde`, `serde_json` as needed |
| File activity | `hookkit-file-activity`, `hookkit-session-state` |
| Runner delegation | `hookkit-tool-runner`, `hookkit-core`, and `clap`; other crates only when directly referenced |

<!-- markdownlint-enable MD013 -->

## 7. Archetypes

An archetype is a named set of defaults, not a second code path. The user still
sees and persists its concrete harness, hook, and state choices. Customizing a
preset changes the resolved modules normally.

Initial library:

<!-- markdownlint-disable MD013 -->

| ID | Default shape | Generated seam |
| --- | --- | --- |
| `policy_guard` | Cross all three, pre-tool, tool-access analysis | Pure `evaluate` returns allow/deny reason; native lowering stays generated |
| `scoped_context_once` | Single harness, pre-tool, session metadata + claim set + tool-access | Discover/match content separately from atomic once-per-session claim |
| `session_bootstrap` | Cross Claude+Codex, aligned session-start + session metadata | `build_context` returns the portable context floor; native extras stay in lowering arms |
| `immediate_quality` | Cross all three, post-tool, Pkl runner | Analyze/fix on all three; Antigravity message output follows the chosen lowering policy |
| `deferred_quality` | Cross all three, file observer + turn completion | Exact start on Claude/Codex; direct evidence and best-effort bootstrap on Antigravity |
| `pre_tool_rewrite` | Cross Claude+Codex, pre-tool rewrite | Pure transform returns replacement input; generated native arms preserve each wire shape |

<!-- markdownlint-enable MD013 -->

`pre_tool_rewrite` is enabled only after the Phase 3B Claude/Codex capability
and lowering tests land. Adding Antigravity makes the preset invalid because
its current pre-tool output cannot replace tool input. An Antigravity
`PreInvocation(invocation_num == 0)` bootstrap may be added later as a clearly
named inferred-invocation archetype; it is not an aligned `SessionStart`.

Archetypes must not copy large example binaries wholesale. They should reuse
library-owned analyzers/runners and generate the smallest domain-specific seam
that users are expected to edit.

### 7.1 Runner archetype configuration

The immediate and deferred archetypes must render a nonempty Pkl configuration;
the runner's default run list is empty. The generated CLI should parse its own
Clap arguments and construct `hookkit_tool_runner::{Cli, FileActivityCli,
SessionStartCli, TurnCompletionCli}`. It should not call the shipped `parse_*`
helpers because their usage text names the shipped binaries.

Until the runner supports a namespaced embedded config, the generated project
should use an explicit, uniquely named Pkl config path in its documented hook
command. It must not silently take ownership of a shared
`.agent-hook-kit/post-tool-use.pkl` in crate mode.

`pkl` is a runtime prerequisite for these two archetypes and must be installed
in their generated CI lane so integration tests do not pass by skipping.

Keep the runner-specific questionnaire focused:

- `quality_profile`: Rust, Python, JavaScript/TypeScript, Go, or custom;
- `quality_tools`: a concrete multiselect defaulted from that profile and
  validated as nonempty;
- `quality_config_path`: default
  `<crate_path>/config/<package_name>.pkl`;
- `lowering_policy`: strict, best effort, or best effort with warnings; the
  generated config persists the exact value;
- deferred-only reconciliation posture: best effort or strict, with best effort
  as the documented default.

Antigravity `PostToolUse` can discover targets and run or auto-fix tools, but
cannot carry the runner's user or agent report. Strict lowering rejects a
nonempty unavailable audience; `best-effort` omits it, and
`best-effort-with-warnings` writes a versioned, collision-safe loss record under
the exact input's `artifactDirectoryPath` and uses exit-zero protocol stderr to
point to it while stdout remains exactly `{}`. Failure to persist that warning
is a hook error rather than a silent drop. At `Stop`, Antigravity has only a
blocking `reason`, so it cannot preserve separate allowed/blocked user/agent
audiences. The manifest and tests must make those losses explicit.

Do not put all 134 embedded builtins into the first top-level menu. Start with a
curated set of well-audited formatter/linter pairs for those profiles plus a
custom tool-spec seam. The rendered Pkl stores the concrete selected tools, so
adding another profile later does not reinterpret an existing answers file.

## 8. State capability library

Expose capabilities in user language while generating the underlying primitive
with explicit family/entity versions.

<!-- markdownlint-disable MD013 -->

| Stable ID | Primitive | Added support |
| --- | --- | --- |
| `session_metadata` | `SessionState::ensure` | Common `--state-dir`; typed session/project metadata |
| `claim_once` | `ClaimSet` | First-writer-wins once-per-session helper |
| `inspectable_set` | `SetJournal<T>` | Typed accumulated set with atomic insert/compaction |
| `record_queue` | `RecordJournal` | Sparse JSON records with snapshot/acknowledge flow |
| `run_artifacts` | `RunBundle` | Relative artifacts plus committed `summary.json` |
| `custom_aggregate` | `EntityJournal<E>` | Advanced typed event/aggregate scaffold and entity mode |
| `file_activity` | `FileActivityStore` and reconciliation | Post-tool producer + completion consumer; exact session start where available |

<!-- markdownlint-enable MD013 -->

Scopes, long-lived exclusive locks, lifecycle observations, and topology
observations belong in the advanced `custom_aggregate` support module or docs,
not as seven more top-level questions.

Generated state constants should be obvious and editable:

```rust
const STATE_FAMILY: &str = "acme-hooks";
const STATE_FAMILY_VERSION: u32 = 1;
```

Every generated schema-bearing entity gets its own visible version constant and
a comment requiring a bump for incompatible event/state changes.

The package-specific default root should be stable across every subcommand and
distinct from other generated packages, for example:

```text
$TMPDIR/agent-hook-kit/generated/acme-hooks/
```

`--state-dir` overrides it. A generated deferred suite passes the resolved path
explicitly to every generated runner entrypoint instead of relying on their
shared global default. Managed dispatch also forwards the override for the
ordinary state helpers through a stable `Option<&Path>` argument on every
generated event-handler seam, including initially stateless projects. Adding a
capability therefore does not change or overwrite a protected handler
signature. When an archetype delegates all selected events to library-owned
runners, custom state capabilities require selecting an additional event with
a user-owned seam; the questionnaire enforces that composition. The exact
session-start entrypoint is registered only for Claude and Codex; all three
receive the post-tool producer and completion consumer.

## 9. Generated project layout and ownership

### 9.1 Full project

```text
<destination>/
  .copier-answers.yml
  .github/workflows/<package>-hook-ci.yml       # optional
  Cargo.toml                                    # workspace
  README.md
  LICENSE-APACHE                                # according to choice
  LICENSE-MIT
  crates/<package>/
    Cargo.toml
    hookkit-template.manifest.yml
    config/                                     # archetype/config dependent
    fixtures/<harness>/<event>.json
    src/
      main.rs
      lib.rs
      scaffold/                                 # Copier-owned adapters
        cli.rs
        dispatch.rs
        harness.rs                              # cross mode only
        lowering.rs                             # when native lowering is needed
        state.rs                                # when selected
      hooks/                                    # user-owned handler seams
        mod.rs                                  # managed module wiring
        aligned.rs / native.rs                  # managed selected exports
        aligned/ / native/                      # protected event seams
        pre_tool.rs / state_types.rs            # managed selected exports
        pre_tool/ / state_types/                # protected policy/type seams
    tests/
      cli.rs
```

Even with one crate, a workspace root gives the project a clean place to add
configuration/support crates later. The CLI crate itself stays identical to
crate-only output.

### 9.2 CLI crate in an existing repository

Run Copier with the existing repository root as the destination so optional CI
lands in the correct place:

```text
<existing repository>/
  .copier-answers.<package>.yml
  .github/workflows/<package>-hook-ci.yml       # optional, unique
  <crate_path>/
    Cargo.toml
    hookkit-template.manifest.yml
    ... same crate tree ...
```

Copier supports templated answer filenames and repeated application to one
destination. Each instance must also own disjoint crate and workflow paths.
Updates to a nondefault answers file use the explicit form:

```text
copier update --answers-file .copier-answers.acme-hooks.yml .
```

Generate that exact command in the crate README or a uniquely named helper
script.

Crate mode must not modify an existing root `Cargo.toml`. It should render the
exact member entry needed when the repository does not already include
`crate_path` through a workspace glob. Automatic TOML mutation would require a
trusted task, create ambiguous file ownership, and make Copier updates more
fragile.

### 9.3 Managed versus user-owned files

- `src/scaffold/**` is template-owned and updated by Copier.
- `src/hooks/*.rs` is template-owned export/dispatch plumbing. Small generated
  handler seams live in `src/hooks/{aligned,native,pre_tool,state_types}/*.rs`
  and are protected with `_skip_if_exists` after first creation. Deselected
  seams remain as harmless orphans so reselecting them restores user work.
- `main.rs` and `lib.rs` remain thin, template-owned wiring.
- policy/config examples intended for user customization are also
  `_skip_if_exists`; changing runner/configuration answers requires explicitly
  reconciling the existing Pkl policy.
- `Cargo.toml`, README sections, fixtures, and CI are template-owned but may
  produce normal smart-update conflicts if the user changed the same lines.

This boundary lets Copier update protocol plumbing without overwriting domain
logic. Adapter-to-handler signatures should be deliberately small and stable;
an incompatible signature change requires a documented migration, not a silent
rewrite of user files.

## 10. Generated Rust design

### 10.1 Dispatch

Always use an explicit event subcommand, even when only one event was selected.
That keeps the CLI stable when another hook is added later.

- A single-harness subcommand calls `run_event::<ExactEvent>`.
- A cross-harness subcommand calls
  `run_aligned_event::<AlignedMarker>(selected_harness, ...)`.
- Pair-specific markers accept only Claude or Codex; the question resolver
  prevents an Antigravity-containing selection from reaching those dispatch
  arms. The deferred-quality support command likewise invokes the precise
  session-start observer only for Claude and Codex and records Antigravity's
  first-observed boundary as best effort.
- Antigravity invocation and tool-use events lack an input discriminator.
  `PreInvocation`/`PostInvocation` remain exact-event dispatch, and explicit
  subcommands select every Antigravity pair without payload guessing.

The handler itself should be a normal function that can be unit tested without
process I/O. In cross mode, domain logic returns a small local decision type and
the generated adapter performs native output lowering.

### 10.2 Safe initial behavior

Every selected event must generate compiling code. The event scaffold catalog
classifies its starter behavior:

- `no_op` — emit the native no-op response;
- `allow` — explicit native allow where appropriate;
- `example` — emit a benign, clearly documented sample value;
- `must_implement` — return a clear handler error for contracts such as a
  required path/result where inventing a value would be unsafe.

Do not install a `must_implement` command into a harness until its handler is
completed. Its generated fixture test should assert the explicit failure,
rather than leaving an accidental `todo!()` panic.

### 10.3 Protocol streams

Generated handler code must not use `println!` or uncontrolled `eprintln!`.
Stdout, stderr, and exit status are contract outputs. User/agent messaging goes
through native output constructors; application diagnostics use a diagnostics
sink or committed run artifacts.

The generated README should put this warning next to the handler seam, not only
in general architecture documentation.

## 11. Event scaffold catalog

Do not derive Rust paths or constructors from event names. They are not
uniform: Claude and Codex divide events differently between `protocol` and
`catalog`, while Antigravity exports at its crate root.

Maintain a curated overlay keyed by exact contract identity:

```yaml
claude-code/docs-2026-08-05-r1/PreToolUse:
  stable_id: pre_tool_use
  rust_event: hookkit_claude::catalog::PreToolUse
  rust_output: hookkit_claude::catalog::PreToolUseOutput
  selector_variant: PreToolUse
  starter: allow
  fixture: fixtures/claude/pre_tool_use.json
  aligned_family: pre_tool
```

The final schema needs at least:

- exact contract, harness, and wire event identity;
- display category/order;
- Rust event and output type paths;
- dynamic selector variant when applicable;
- initial-output strategy/expression or fragment;
- fixture path and required environment fixture;
- aligned-family mapping, if any;
- semantic capability flags, including input rewrite, true pre-action block,
  post-action feedback, ignored/advisory output, tool-result availability,
  separate user/agent messages, and exact/inferred session boundary;
- scaffold support status and explanatory note.

Keep alignment records in `alignment-families.yml`, separate from exact-event
scaffolds. Each record declares its stable ID, implemented sealed marker, exact
native member per harness, supported-harness set, portable input/output floor,
and capability intersection. The catalog checker rejects an alignment whose
marker cannot compile or whose native contracts are absent from the registry.

Add an `xtask` command that joins this overlay to
`contracts/status/implementation/registry.json` and fails unless every
implemented command event has exactly one scaffold record. It should generate
the single-harness question choices and check for stale records. Compilation of
the generated matrix remains the authority for Rust path and constructor drift.

The refreshed catalog needs explicit regression cases for Claude
`DirectoryAdded`, Codex `SessionEnd`, Claude `SessionStart(source = "fork")`,
and Antigravity's typed post-tool call. Codex `SessionEnd` must start with empty
stdout because its output is ignored. Claude `PostToolUse` exit 2 is feedback
after a completed action, never a blocking capability. Both Claude and Codex
pre-tool scaffolds record input rewriting; Antigravity records
`deny_unless_prior_grant` as a native-only decision. The overlay also retains
the refreshed native details such as typed Antigravity injected steps and
Claude worktree paths without trying to normalize them into alignments.

The template catalog is a consumer-facing presentation layer, not a second
protocol ledger. Contract identity and implementation status stay sourced from
`contracts/`; only ergonomic Rust/scaffold metadata is curated here.

## 12. Dependency source and publication transition

### 12.1 Before publication

Default every HookKit crate to the same repository and exact Git revision:

```toml
hookkit-core = {
  git = "https://github.com/prb/agent-hook-kit",
  rev = "<full-sha>",
}
hookkit-runtime = {
  git = "https://github.com/prb/agent-hook-kit",
  rev = "<same-full-sha>",
}
```

Centralize this rendering in one Jinja macro/partial. Never default to
`branch = "main"`. The compatibility catalog contains the tested revision, and
template CI rejects an unpinned Git dependency.

Path mode exists for this repository's render tests and local contributors. A
generated project that uses it is visibly nonportable.

### 12.2 After publication

Change the public default to one compatible crates.io version in the template's
compatibility catalog. Keep Git and path modes for development. The generated
binary project should commit `Cargo.lock` after its first setup check.

Publication must include the complete 12-crate graph used by every advertised
archetype, not only the seven crates in the current release checklist. Use one
compatibility/release train and verify this staged order:

1. `hookkit-core`;
2. `hookkit-claude`, `hookkit-codex`, `hookkit-antigravity`,
   `hookkit-session-state`, and `hookkit-pkl-config`;
3. `hookkit-common` and `hookkit-shell`;
4. `hookkit-tool-access`;
5. `hookkit-file-activity`;
6. `hookkit-runtime`;
7. `hookkit-tool-runner`.

`hookkit-runtime` could publish earlier from its normal dependency edges, but
placing it after file activity permits complete package verification with its
current internal development dependencies. Generate the authoritative order
from Cargo metadata, document it in `RELEASE.md`, extend release/MSRV checks to
all 12 crates, and run `cargo package --list` plus staged package verification
for each publishable package.

Do not switch the public default to crates.io until every crate required by an
offered archetype is published at the compatible version. A partial publication
must instead leave Git as the default and mark unavailable archetypes, rather
than mixing dependency sources invisibly.

### 12.3 Later compile-graph improvement

Add per-harness features to `hookkit-common` and `hookkit-runtime`, stop forcing
`hookkit-shell/all-harnesses` through `hookkit-tool-access`, and forward the
selection through `hookkit-file-activity` and `hookkit-tool-runner`. Once those
APIs exist, the existing selection resolver can emit the correct features
without changing the questionnaire or archetypes.

## 13. Optional GitHub Actions component

Generate exactly one workflow owned by the package:

```text
.github/workflows/<package-name>-hook-ci.yml
```

Use names derived from the package, not generic `CI` identifiers:

```yaml
name: <project name> hook CI
jobs:
  <package_snake>_hook_checks:
```

The workflow should:

- grant `contents: read` only;
- use a workflow/PR-aware concurrency group and cancel superseded PR runs;
- install Rust 1.85 for an MSRV `cargo check`;
- install stable with rustfmt/clippy;
- run format, clippy with `-D warnings`, and tests against the generated
  package/manifest;
- install a compatibility-catalog-pinned Pkl version only when a runner
  archetype requires it;
- use Ubuntu by default and add macOS to the test matrix for full projects;
- avoid path filters in crate mode initially, because generic path filters
  cannot know every shared workspace input.

In crate mode, commands use `--manifest-path <crate_path>/Cargo.toml`; once the
crate is a workspace member the repository may prefer `-p <package>`. The
workflow filename, workflow display name, job ID, cache key, and artifact names
must all be package-namespaced.

Copier has no task-free `_fail_if_exists` setting. On an interactive initial
copy it prompts before replacing a differing file, with overwrite as the
default answer; in a noninteractive copy it fails. Therefore crate-mode CI is
off by default, the generated instructions must say not to use `--overwrite`
against an existing repository, and they must include an explicit preflight for
the computed workflow path. A collision means answer **no** and choose another
package/workflow name. Do not use `_skip_if_exists`: that would protect the
first copy but also prevent future template updates to the owned workflow.

If a later first-party launcher wraps Copier, it should make this preflight
fail closed before any destination file is rendered. The workflow-name
namespace is the primary v1 collision avoidance; direct Copier alone cannot
provide an atomic no-collision guarantee.

## 14. Update behavior

Smart updates require:

- a committed Copier answers file;
- a Git-versioned generated destination;
- immutable, PEP 440-compatible template tags;
- deliberate ownership boundaries;
- a real-tag upgrade test once a second public template version exists.

For full projects, the conventional `.copier-answers.yml` allows ordinary
`copier update`. For repeated crate-mode applications, use a templated
`.copier-answers.<package>.yml` and always supply `--answers-file` to update,
recopy, and `check-update`.

Normal smart diffing should handle most changes. Add migrations only for an
answer rename, a managed path move, or another transition that cannot be
expressed safely as old-template/new-template diff. Migrations and tasks are
unsafe Copier features and require trust; introducing either is an explicit
template security decision.

The generated CI or repository maintenance workflow may run:

```text
copier check-update --answers-file .copier-answers.<package>.yml --quiet .
```

It should report availability, not automatically rewrite a user's branch.
Template-repository CI pins the exact Copier version used for render snapshots;
generated maintenance instructions use the same compatible pin instead of an
unbounded `pipx install copier`.

## 15. Template test strategy

### 15.1 Render/compile matrix

At minimum, CI renders and validates:

1. full project, a crate in a non-workspace repository, and a crate in an
   existing Cargo workspace;
2. CI disabled and enabled, paired across topologies rather than multiplied by
   every other answer;
3. one maximal single-harness render for each of Claude's 31, Codex's 11, and
   Antigravity's 5 current command events, with counts derived from the
   registry;
4. all four cross subsets: Claude+Codex, Claude+Antigravity,
   Codex+Antigravity, and all three;
5. every current universal family on all three, every richer family on
   Claude+Codex after Phase 3B, and negative assertions that those richer
   families disappear from every Antigravity-containing set;
6. every state capability and archetype at least once in each advertised
   fidelity class, including Antigravity post-tool evidence, inferred session
   bootstrap, and message-lowering behavior;
7. local path compilation, a clean Git-source smoke, and a clean crates.io
   smoke once published;
8. two crate-mode invocations into one repository with disjoint answers,
   crates, and workflows;
9. negative cases for one-harness cross mode, invalid family/archetype/state
   combinations, escaping paths, incomplete Git SHAs, duplicate owned paths,
   and strict Antigravity output requirements.

This is catalog-exhaustive plus pairwise/high-risk coverage, not a Cartesian
product of every question.

For each applicable render, run:

```text
cargo fmt --check
cargo +1.85.0 check --all-targets
cargo check --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Use path dependencies in repository CI so the generated code is tested against
the exact checkout. Also smoke a clean generated project against the public,
reachable compatibility Git SHA, with the same SHA on every HookKit dependency
and no branch reference. Pull-request CI may syntax-check Git rendering before a
new commit is public, but release CI must perform the real fetch. After
publication, run `cargo check --locked` in crates.io mode with no workspace
patch or path escape and inspect the resolved dependency sources.

For an existing Cargo workspace, assert Copier leaves the root manifest
byte-for-byte unchanged, then simulate the documented manual member registration
before compiling. Cover a non-workspace host separately; a child manifest alone
can be rejected when Cargo believes it belongs to an unmodified parent
workspace. Pin Copier and Pkl in these tests.

### 15.2 Protocol tests in generated projects

- Unit-test pure handler/domain functions.
- Include one native fixture per selected harness/event.
- Use `execute_typed` or `execute_aligned_event` with explicit environment maps
  for deterministic stdout/stderr/exit assertions.
- Loop across every selected arm for a cross-harness policy.
- Assert Codex `SessionEnd` emits empty stdout, Claude's fork session source
  parses, and Claude post-tool exit 2 is classified as feedback rather than a
  pre-action block.
- Prove Antigravity tool-call arguments produce direct file evidence and that
  empty post-tool output rejects or records unavailable messages according to
  the selected lowering policy.
- For `claim_once`, invoke twice and assert action then no-op.
- For runner archetypes, use hermetic fake executables in the default test lane.
- Do not duplicate HookKit's deep crash/concurrency proofs in every generated
  project.

### 15.3 Template integrity checks

- Event overlay and the current command registry are a one-to-one match;
  alignment records refer only to implemented markers and registered members.
- No unrendered Jinja delimiters remain.
- Generated paths never escape the destination.
- Crate mode leaves pre-existing root files byte-for-byte unchanged except its
  unique answers file and explicitly selected unique workflow.
- Every rendered direct dependency corresponds to a generated source use, and
  every generated external crate path has a direct manifest dependency; compare
  `Cargo.toml`, the resolved manifest, and source in both directions.
- Every selected event is reachable from CLI dispatch.
- Every generated workflow parses as YAML and uses namespaced identifiers.
- Every Git dependency uses the same full, reachable revision; no default
  dependency follows a mutable branch.
- Only the three supported harness IDs appear in generated choices or
  manifests.

The repository suite already simulates immutable v1.0.0 and v1.1.0 template
tags, verifies that `src/hooks/**` edits survive, and confirms managed files
advance without unexpected rejects. After a second public release exists, add
the equivalent remote tag-to-tag smoke.

## 16. Implementation record

### Phase 0 — green and lock the baseline (complete)

- Confirm repository CI and current documentation remain free of removed-harness
  references and obsolete Antigravity post-tool assumptions.
- Regenerate/run the current contract, conformance, MSRV, package, and workspace
  checks before adding template jobs.
- Derive the initial 47-event scaffold baseline from the implementation
  registry and lock the selected-set capability/fidelity matrix.
- Review and approve the eight Claude/Codex portable floors before implementing
  their common/runtime APIs.

### Phase 1 — template foundation (complete)

- Add root Copier configuration, modular questions, and two output topologies.
- Add identity/path validation and unique answer-file behavior.
- Add dependency-source rendering with an exact Git revision.
- Generate a minimal branded CLI, README, manifest, and optional namespaced CI.
- Prove one exact hook and the current all-three aligned pre-tool hook end to
  end.

### Phase 2 — complete event catalog (complete)

- Add the curated scaffold overlay for every implemented command event.
- Add the contract/overlay `xtask` check.
- Generate native hook choices, handler seams, fixtures, and safe starter
  outputs for all 47 current events across the three harnesses.
- Compile one all-events project per harness.

### Phase 3 — cross-harness modules (complete)

- **3A, current universal tier:** add the three existing aligned families,
  selected-set resolution, native lowering fragments, fidelity-aware
  validation, and all four cross-subset cases.
- **3B, richer pair tier:** add lossless common wrappers and sealed runtime
  specs for `PermissionRequest`, `PreCompact`, `PostCompact`, `SessionStart`,
  `SessionEnd`, `SubagentStart`, `SubagentStop`, and `UserPromptSubmit`; prove
  every portable floor against both exact contracts.
- Add the Claude/Codex pre-tool rewrite lowering and ensure both the richer
  families and rewrite capability disappear when Antigravity is selected.
- Generate alignment choices only from the implemented alignment catalog; no
  Phase 3B choice may precede its library marker.

### Phase 4 — state capabilities and archetypes (complete)

- Add session metadata, claims, set, queue, run bundle, and aggregate modules.
- Add package-specific state roots.
- Add file-activity compound selection and the six initial archetypes.
- Exercise exact session start on Claude/Codex and direct post-tool evidence
  with inferred bootstrap on Antigravity.
- Add generated Pkl config and non-skipping runner tests.

### Phase 5 — update hardening (local work complete; release tag pending)

- Establish managed/user-owned file policies.
- Add repeated crate-instance tests.
- The acceptance suite already exercises a synthetic copy-from-v1/update-to-v1.1
  transition; tag the first public template release after its clean Git smoke.
- Document `copy`, `update`, `recopy`, and `check-update` commands.

### Phase 6 — public crate switch (pending publication)

- Bring all 12 template-facing crates into one compatibility/release train and
  publish them in the verified dependency order.
- Update the compatibility catalog to default to crates.io.
- Extend package/release/MSRV checks to every crate used by an archetype and run
  the clean crates.io generated-project smoke.
- Keep exact Git revision and path modes as explicit development choices.

## 17. Acceptance criteria

### 17.1 Git-pinned preview gate

The pre-publication template preview is ready when:

- a user can generate either topology without `--trust`;
- presets and custom mode persist concrete, reviewable choices;
- the overlay exactly covers all 47 current command events, and single mode
  exposes every one;
- cross availability equals the selected-set intersection and never offers a
  family absent from the sealed aligned runtime;
- Claude+Codex offers the universal three plus the eight approved pair
  alignments, while every Antigravity-containing set offers only compatible
  families/capabilities;
- every generated selection compiles or, for a required domain result, fails at
  runtime with an explicit `must implement` error rather than a panic;
- direct dependencies and generated source agree in both directions;
- state capabilities generate only their required crates/modules;
- file activity always generates its coordinated commands and one unique state
  root, with exact versus inferred session-boundary fidelity recorded;
- unavailable Antigravity output semantics fail during validation under strict
  requirements or are explicitly recorded under a chosen best-effort policy;
- two crate templates coexist in one repository without clobbering answers,
  source paths, state, config, or CI;
- both non-workspace and Cargo-workspace crate hosts follow tested integration
  instructions without Copier mutating their root manifest;
- generated handler edits survive a tested Copier update;
- CI names and files are package-specific and the generated workflow passes;
- Rust 1.85 and stable generated-project lanes pass;
- every pre-publication dependency is pinned to the same public, reachable Git
  revision.

### 17.2 Public v1 gate

Public v1 additionally requires all 12 advertised crates at one compatible
crates.io version and a clean, locked generated-project check whose resolved
dependencies come only from crates.io. The compatibility catalog switches its
default only after that smoke passes; Git and path remain explicit development
choices.

## 18. Implemented decisions

The implementation uses these defaults:

1. **Supported set:** Claude Code, Codex, and Antigravity are the complete
   built-in set; cross mode selects all three by default.
2. **Alignment depth:** the eight additional Claude/Codex families are backed
   by common/runtime support; Antigravity-containing sets keep the compatible
   intersection.
3. **Binary shape:** one binary with stable event subcommands, not one binary
   per event.
4. **Template shape:** one conditional Copier template, not several chained
   templates.
5. **Crate-mode safety:** do not mutate an existing root `Cargo.toml`; render
   the required member entry.
6. **Copier security:** no tasks or migrations in v1, hence no `--trust`.
7. **Session fidelity:** file activity supports all three; exact start is
   Claude/Codex and Antigravity bootstrap is explicitly best effort.
8. **Dependency bodge:** one exact Git revision now; crates.io becomes the
   public default only after the full 12-crate graph is published and smoked.
9. **Initial archetypes:** policy guard, once-per-session scoped context,
   session bootstrap, immediate quality, deferred quality, and Claude/Codex
   pre-tool rewrite.

None of these prevents later expansion. Changes to these boundaries should use
normal Copier update semantics, with an explicit migration only when an answer
key or managed path cannot change safely through smart diffing.
