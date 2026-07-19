# hookkit-tool-access

`hookkit-tool-access` converts an observable native tool call into
phase-agnostic, loss-aware file-access evidence. It retains structured JSON
field provenance, patch operation roles, shell inference details, lexical path
resolution, certainty, and typed gaps.

The analyzer is static evidence, not a sandbox or audit log. It does not execute
tools, expand runtime shell values, canonicalize paths, follow symlinks,
materialize globs, or promise that every runtime access is visible. Consumers
decide how to treat unresolved or unclassified evidence.

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
