# hookkit-tool-access

`hookkit-tool-access` converts an observable native tool call into
phase-agnostic, loss-aware file-access evidence. It retains structured JSON
field provenance, patch operation roles, shell inference details, lexical path
resolution, certainty, and typed gaps.

The analyzer is static evidence, not a sandbox or audit log. It does not execute
tools, expand runtime shell values, canonicalize paths, or promise that every
runtime access is visible. Consumers decide how to treat unresolved or
unclassified evidence.

The `codex-claude-rules` and `forbidden-file-guard` examples are complete
pre-tool consumers: the former expands referenced directories before selecting
rules, while the latter demonstrates inspect-known and fail-closed policy
postures across all three aligned harness arms. `hookkit-file-activity` uses the
same analyzer after tool execution and deliberately keeps only modification
evidence.

Aligned `PreToolUseInput` and `PostToolUseInput` values can be analyzed directly:

```rust
# use hookkit_common::PreToolUseInput;
# fn inspect(input: &PreToolUseInput) {
let report = hookkit_tool_access::ToolAccessAnalyzer::default().analyze_pre_tool(input);
for candidate in report.may_read() {
    eprintln!("possible read: {}", candidate.provenance);
}
if !report.is_complete() {
    eprintln!("the observable call could not be analyzed completely");
}
# }
```

Typed single-harness hooks can analyze their native input in place with
`ToolAccessAnalyzer::analyze_native`, which accepts any `ObservableToolCall`
(aligned inputs, Codex and Antigravity pre/post inputs, and Claude catalog
inputs for `PreToolUse`, `PermissionRequest`, `PostToolUse`, and
`PostToolUseFailure`) without cloning it into an aligned wrapper.

## Built-in tool contracts

Documented harness built-ins are analyzed from exact per-harness
`(tool name, JSON Pointer)` contracts rather than the generic key heuristics,
so a PascalCase Antigravity `TargetFile` is recognized without making the same
key a path for unrelated MCP tools:

| Harness | Tool | Argument | Intent and scope |
| --- | --- | --- | --- |
| Claude Code | `Read` | `/file_path` | read, exact |
| Claude Code | `Write` | `/file_path` | modify, exact |
| Claude Code | `Edit`, `MultiEdit` | `/file_path` | read-modify, exact |
| Claude Code | `NotebookEdit` | `/notebook_path` | read-modify, exact |
| Claude Code | `Grep` | `/path` (default cwd), `/glob` | read, root and descendants (or each `glob` filter below the root) |
| Claude Code | `Glob` | `/path` (default cwd), `/pattern` | enumerate, glob below the root |
| Codex | `apply_patch` | `/command` (patch text) | per patch header |
| Codex | `view_image` | `/path` | read, exact |
| Antigravity | `view_file` | `/AbsolutePath` | read, exact |
| Antigravity | `write_to_file` | `/TargetFile` | modify, exact |
| Antigravity | `replace_file_content`, `multi_replace_file_content` | `/TargetFile` | read-modify, exact |
| Antigravity | `list_dir` | `/DirectoryPath` | enumerate, exact |
| Antigravity | `find_by_name` | `/SearchDirectory`, `/Pattern` | enumerate, glob below the directory |
| Antigravity | `grep_search` | `/SearchPath`, `/Includes` | read, root and descendants (or each include below the root) |
| Antigravity | `generate_image` | `/ImagePaths` | read, exact |

A repository-wide `Grep` therefore materializes every descendant of the
working directory; a secret-file policy denies it unless the search is narrowed
with `path` or `glob`. Claude splits a `Grep` `glob` into several ripgrep
filters (on whitespace, then on commas outside a `{...}` group), so
`*.md,.env` records both `**/*.md` and `**/.env`; a negated filter falls back
to the whole root. Claude passes a `Glob` pattern to `rg --files --glob`, so a
pattern without `/` matches at any depth below the root, an absolute pattern
is searched from its literal directory, and a pattern that repeats the end of
the search directory (`web/src/*.ts` below `web`) is also recorded anchored
below it, as Claude rewrites it when the repeated directories do not exist.

Tools documented not to touch files (for example Claude `WebFetch`,
`WebSearch`, `TodoWrite`, `Agent`, `AskUserQuestion`, and `ExitPlanMode`;
Codex `update_plan`, `spawn_agent`, and `web_search`; Antigravity
`search_web`, `read_url_content`, and `schedule`, and `manage_task` with the
`list`, `status`, or `kill` action) yield an empty, complete report. Tools that
may reach files their arguments do not name, such as Codex `write_stdin` and
`read_mcp_resource` (whose URI may be `file://`) or Antigravity `manage_task`
`send_input`, record a gap.
`with_builtin_tools(false)` restores heuristic-only analysis.

Other tools use `StructuredFieldAnalyzer`, which classifies a tool by whole
words in its name (`remove_file` deletes, `download_file` writes, and a
run-together word joining a verb and a file-system noun, such as `writefile`,
counts too). Move- and copy-like tools consult every source/destination key
(`source`, `destination`, `from`, `to`, ...); tools that write also consult the
destination-style keys (`destination`, `dest`, `output`, `output_path`,
`target_path`, ...) and tools that read the source-style ones (`source`,
`src`, `old_path`), but never the generic `from` and `to`, which calendar and
similar tools use for other values. A structured path beginning with `~` is
left unresolved with an `UnexpandedHomePath` gap because home expansion is up
to the tool.

## Patches and working directories

Codex sends `apply_patch` hook input as `{"command": "<patch>"}`; custom patch
tools may use `patch` or `input`. The parser follows Codex's grammar (headers
are recognized on trimmed lines outside update hunks, and hunk lines are never
headers), unwraps Codex's lenient `<<'EOF'` wrapper, and leaves paths
unresolved with an `UnknownExecutionEnvironment` gap when a patch names
`*** Environment ID:`. Unified diffs honor `@@` line counts, so removed or
added lines that begin with `-- ` or `++ ` are content, not headers.

`ToolAccessAnalyzer::with_shell_profile` and `with_shell_profiles` opt exact
custom shell shapes into the same analysis without speculative aliases. The
contract-backed profile constants from `hookkit-shell` avoid repeating native
field pointers. Shell `apply_patch` (or `applypatch`) commands, including ones
run through a wrapper such as `command`, `env`, `nohup`, `sudo`, or `timeout`,
are fed through the shared patch parser and retain both shell source spans and
patch-operation provenance. The patch text is what the standalone
`apply_patch` executable reads: its single argument when it has one
(`ShellPatchArgument` provenance), and otherwise its final standard-input
redirection, or that of an enclosing statement as in
`(cd dir && apply_patch) <<'EOF'` and `{ apply_patch; } <<'EOF'`. Only a
here-document there is analyzed; a later `< file`, `<>`, or `<&-`, a pipe, a
here-document on another descriptor, or a dynamic or extra argument records a
`MissingShellPatchHereDocument` gap instead of trusting a decoy.

Codex applies a script that is exactly `apply_patch <<EOF` or
`cd <dir> && apply_patch <<EOF` itself, from the raw here-document body, so in
that form header paths free of shell syntax are exact even when an unquoted
body's hunks contain `$`. Any other script (anything before or after it, and
every Claude or Antigravity command) runs in Bash, which expands an unquoted
body before `apply_patch` reads it and can inject file headers that no literal
line shows: literal headers are still recovered, but a
`DynamicShellPatchHereDocument` gap is always recorded. Dynamic header paths,
partial Bash analysis, and cwd uncertainty remain typed gaps too.

A `cd <dir> && apply_patch` chain (including `cd a && cd b && ...`) that
begins its list resolves paths against the working directory joined with each
literal `cd` operand. A non-literal operand, or a script that mentions
`CDPATH` or `cdable_vars` and runs in Bash, leaves them unresolved and
`Heuristic`; any other directory change that may take effect first (a wrapped
`builtin cd`, a negated `! cd dir`, a `cd` skipped by `||`, one in a loop or
function body, or a wrapper option such as `env -C`) leaves them unresolved.
Both record a `ShellPatchWorkingDirectoryMayHaveChanged` gap.

Relative paths carry the basis they were resolved against. `InvocationCwd`
means the tool's own working directory, including a Codex `workdir` or an
Antigravity `Cwd` argument (a relative one is joined onto the hook's
directory). `SessionCwd` means only the hook's session directory was
observable: Codex `Bash` payloads omit the `workdir` argument that can move the
command elsewhere (`ShellCwdOrigin::UnverifiedFallback`), and Antigravity
structured tools are resolved against the first workspace root. A shell path
beginning with `~` is kept as a `Home` candidate with its raw `~/...` text; it
is resolved only when the analyzer is built with `with_home`, and its
dependence on the shell's `$HOME` stays a gap either way. Without a working
directory (for example an Antigravity payload with empty `workspacePaths`),
relative paths are reported with a missing-working-directory gap rather than
dropped.

## Bounded target materialization

`resolve_targets` accepts unified `AccessTarget` values directly, so pre-tool
policy does not need to construct a file-activity aggregate. Its
`TargetResolutionOptions` explicitly controls workspace roots, ignored and
excluded directories, entry budget, symlink traversal, nonexistent exact
paths, and whether I/O/glob failures are reported or abort resolution.

```rust
# use hookkit_tool_access::{ExactPathPolicy, TargetResolutionOptions};
# fn materialize(report: &hookkit_tool_access::ToolAccessReport) {
let mut options = TargetResolutionOptions::new(vec!["/workspace".into()]);
options.exact_paths = ExactPathPolicy::RetainNonexistent;
let resolved = hookkit_tool_access::resolve_targets(
    report.may_modify().map(|candidate| &candidate.target),
    &options,
).expect("report mode keeps target-local failures in the result");
for path in resolved.paths {
    eprintln!("possible target: {path}");
}
# }
```

The resolver performs a bounded filesystem snapshot. It is not symbolic glob
intersection proof: racing filesystem changes can alter results, and every
target skipped after budget exhaustion is reported as unresolved. Use
`ExactPathPolicy::RetainNonexistent` for pre-tool create/write checks and the
default `ExistingOnly` mode for deferred execution over existing files.

Ignored directory names prune traversal only: an exact target such as
`.git/hooks/pre-commit`, or a file that happens to be named `target`, still
materializes. Under the default `SymlinkPolicy::DoNotFollow`, a descendant
scope whose root is a directory symlink yields the link itself rather than the
linked tree; a glob's literal prefix is still followed, as a shell would. Brace
alternation (`{src,tests}/*.rs`) ends a glob's literal root. A relative glob
whose base is unknown (after a directory change, without a working directory,
or in another environment) is reported as an unresolved expression instead of
being guessed against the workspace roots.
