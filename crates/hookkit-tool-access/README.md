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
postures across all four aligned harness arms. `hookkit-file-activity` uses the
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

`ToolAccessAnalyzer::with_shell_profile` and `with_shell_profiles` opt exact
custom shell shapes into the same analysis without speculative aliases. The
contract-backed profile constants from `hookkit-shell` avoid repeating native
field pointers. Literal, statically delimited shell `apply_patch` heredocs are
fed through the shared patch parser and retain both shell source spans and
patch-operation provenance; dynamic bodies, partial Bash analysis, and cwd
uncertainty remain typed gaps.

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
