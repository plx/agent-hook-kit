# Post-tool-use runner design

The runner owns tool selection, recursive file discovery, Pkl policy, execution,
classification, diagnostics artifacts, and the `RunnerDomainOutcome` semantic
result. Core, native, common, and runtime crates do not depend on those policies.

Per-harness lowering is intentionally explicit. Claude, Codex, and Gemini CLI use
their native capabilities through the compatibility adapter while migration to
lossless native arms continues. Antigravity `PostToolUse` contains a step index and
optional error but no tool call, tool result, or changed-file list, so this runner
does not claim Antigravity support and must not infer those fields.

To add a tool, extend the Pkl catalog and its fake-executable orchestration cases;
real-tool fixtures belong to the opt-in compatibility lane. To add a harness,
first prove how its native event yields changed files, then add an explicit final
lowering arm and exact native fixture tests.

Performance budgets for the stabilization release are measured, not protocol
guarantees: startup and clean no-op paths are recorded by the hermetic smoke lane;
tool subprocess time is reported separately from parsing/dispatch overhead.
