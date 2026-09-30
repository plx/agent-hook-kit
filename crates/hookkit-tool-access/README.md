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
| Claude Code | `Grep` | `/path` (default cwd), `/glob` | read, root and descendants (or the `glob` below the root) |
| Claude Code | `Glob` | `/path` (default cwd), `/pattern` | enumerate, glob |
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
with `path` or `glob`. Tools documented not to touch files (for example Claude
`WebFetch`, `WebSearch`, `TodoWrite`, `Agent`, `AskUserQuestion`, and
`ExitPlanMode`; Codex `update_plan`, `spawn_agent`, and `web_search`;
Antigravity `search_web`, `read_url_content`, `manage_task`, and `schedule`)
yield an empty, complete report. `with_builtin_tools(false)` restores
heuristic-only analysis.

Other tools use `StructuredFieldAnalyzer`, which classifies a tool by whole
words in its name (`remove_file` deletes, `download_file` writes) and consults
source/destination keys (`source`, `destination`, `from`, `to`, ...) only for
move- or copy-like tools. A structured path beginning with `~` is left
unresolved with an `UnexpandedHomePath` gap because home expansion is up to the
tool.

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
field pointers. Literal, statically delimited shell `apply_patch` (or
`applypatch`) heredocs are fed through the shared patch parser and retain both
shell source spans and patch-operation provenance. In an unquoted heredoc whose
hunks contain `$`, header paths free of shell syntax are still recovered, since
Codex applies the body verbatim; dynamic header paths, partial Bash analysis,
and cwd uncertainty remain typed gaps.

Relative paths carry the basis they were resolved against. `InvocationCwd`
means the tool's own working directory. `SessionCwd` means only the hook's
session directory was observable: Codex `Bash` payloads omit the `workdir`
argument that can move the command elsewhere, and Antigravity structured tools
are resolved against the first workspace root.

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
