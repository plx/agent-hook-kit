# ADR 016: Command-hook environments are explicit native inputs

- Status: accepted
- Date: 2026-07-13
- Phase: 8

## Context

Several harnesses convey invocation state through environment variables in
addition to JSON on stdin. Reading `std::env` inside handler bodies hides that
input, makes tests depend on process-global mutation, and invites ambient
credentials or user configuration into the protocol model. The environment is
also specific to command handlers: HTTP, prompt, agent, and MCP bindings do not
share a command subprocess contract.

## Decision

`hookkit-core` owns a deterministic `EnvironmentVariables` map and the
`CommandEnvironmentSpec` parsing contract. Every native harness owns one
structured command-environment type, including an explicit zero-field type when
its official contract defines no variables. `EventSpec` and `HarnessSpec`
associate that type with their native input and output.

The stdin/stdout `run_*` adapters capture only exact names and prefixes declared
by the selected environment spec. The deterministic `execute_*` APIs instead
receive an injected `EnvironmentVariables` map. Both paths use the same parser,
validate redundant JSON/environment values before dispatch, and pass the parsed
environment to the handler as a first-class second argument.

Full inherited environments and arbitrary per-handler configured variables are
outside the typed contract. They remain available to the actual child process
according to harness behavior, but cannot identify an event and are not copied
into `RuntimeContext`. Debug representations of raw maps and dynamic Claude
plugin options redact values.

## Consequences

- Hook bodies can express all declared invocation inputs in their signatures and
  test them without mutating the process environment.
- Exact, selected-harness, runtime-selected, and aligned execution retain native
  environment types rather than introducing a lossy common bag.
- Downstream `EventSpec` and `HarnessSpec` implementations must add a
  `CommandEnvironment` associated type; `NoCommandEnvironment` supports events
  with no modeled harness state.
- Existing two-argument handlers become three-argument handlers, and in-memory
  execution calls add an explicit variable-map argument. This is an intentional
  pre-1.0 breaking change.
- Newly documented upstream variables require a contract and parser update;
  incidental ambient or undocumented variables do not expand the API by
  observation alone.
