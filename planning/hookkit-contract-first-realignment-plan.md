# Agent Hook Kit: Contract-First Realignment Plan

<!-- markdownlint-disable MD013 MD024 -->

- Status: proposed controlling implementation plan
- Prepared: 2026-07-12
- Repository baseline: `a997d480a62245f6718cd44b9c7f50d52b635612`
- Initial library version: `0.1.0`

Scope amendment (2026-07-15): ADR 018 supersedes this plan's earlier
non-command binding policy. HookKit implementation work is limited to command
bindings in the current iteration. Non-command bindings remain catalogued as
upstream evidence but are explicit unsupported targets, not implementation
backlog.

## 1. Status, authority, and how to use this plan

This document turns the July 2026 repository audit and the clarified product intent into an implementation plan that can be executed across multiple sessions and pull requests.

If this plan is adopted, it supersedes older files in `planning/` wherever they conflict with it. Those files remain useful historical context; they are not instructions to preserve an API or assumption that the current audit found to be wrong. In particular, this document controls over plans that assume:

- automatic harness detection is the normal execution path;
- hook event inference must always occur at runtime;
- Gemini CLI and Antigravity are one protocol;
- one generic output envelope can safely represent every event;
- a common output model may discard native harness capabilities; or
- the post-tool-use runner should be extracted before the core library is stable.

Implementation agents should:

1. identify the phase and work package they are implementing;
2. read the controlling decisions and exit gate for that phase;
3. consult the contract catalog rather than implementing a protocol from memory;
4. keep the work package small enough to review as one coherent protocol change;
5. update every applicable layer—catalog, fixtures, Rust, tests, and support
   status—without allowing protocol implementation to precede its reviewed
   contract; and
6. leave a handoff using the checklist in section 16.

This is an architectural remediation plan, not authorization to treat every illustrative Rust or YAML spelling below as frozen. Public semantics and safety properties are controlling. Exact names may change during the Phase 1 format spike or the Phase 2 API spike if the replacement is documented and preserves those properties.

## 2. Executive determination

The project should be remediated, not rewritten from scratch and not left as-is.

The original concept remains sound: a Rust library can make sophisticated hooks far easier to write while retaining each harness's native input, output, and process semantics. The existing workspace already has the right broad pieces:

- a small shared core;
- native crates per harness;
- a runtime crate;
- lossless wrapper enums for some aligned inputs;
- examples; and
- a substantial real consumer that exercises post-tool-use behavior.

The problem is at the protocol boundary. Several native models describe older or inaccurate contracts, output types permit invalid event/output combinations, the common output layer normalizes too early, and tests rely too much on synthetic payloads and local external-tool behavior. Codex and Gemini CLI require substantial contract rewrites. Claude requires a complete event audit and an output rewrite. Antigravity needs its own crate and identity.

The recommended target is:

1. maintain a versioned repository-side inventory of every harness/event input and output contract;
2. make native event types the authoritative Rust API;
3. make valid output depend on both event and handler transport;
4. require explicit harness selection on dynamic execution paths;
5. retain inference as a separate diagnostic/resolution capability, with optional event hints;
6. wrap aligned native inputs and outputs losslessly for multi-harness hooks;
7. defer a stable common “semantic intention” layer until real consumers prove it is faithful; and
8. retain the post-tool-use runner as a north-star acceptance consumer until the library stabilizes.

## 3. Current situation and disposition

The audit baseline is useful but not release-ready:

- `cargo fmt --check` and strict Clippy pass.
- The normal test selection passes 236 tests.
- The full workspace suite has six environment-sensitive runner golden failures caused by external tool/version behavior.
- The Pkl configuration and post-tool runner account for roughly 90% of tracked repository files.
- Internal publication metadata and package verification are incomplete.

These facts are a baseline, not acceptance criteria. Phase 0 must re-record them in a durable, reproducible form.

| Area | Current assessment | Disposition |
| --- | --- | --- |
| Workspace/crate decomposition | Broadly appropriate | Keep and refine |
| `hookkit-core` primitives | Useful but too weak for exact event identity | Refactor |
| Native input wrapper approach | Directionally correct and often lossless | Keep, then rebuild from contracts |
| Claude input model | Useful base but incomplete | Re-audit and remediate |
| Claude output model | Can emit event-invalid payloads | Rewrite |
| Codex model | Stale event set and wire assumptions | Rewrite |
| Gemini CLI model | Current names exist, but wire shapes and outputs are stale | Rewrite |
| Antigravity support | Missing and incorrectly conflated with Gemini in the old product story | Add a distinct crate |
| Generic runtime plumbing | Valuable | Refactor around typed events and exact process emission |
| Harness auto-detection | Over-emphasized | Retain only as optional inspection capability |
| Hook-event inference | Useful only on dynamic paths | Make optional; concrete hooks are statically typed |
| Common input enums | Match the native-fidelity goal | Preserve and expand |
| Common output abstraction | Loses native semantics | Replace with native-arm enums |
| Common semantic intentions | Unproven | Keep hook-local initially; promote only by evidence |
| Post-tool-use runner | Overwhelms repository, but is a critical design consumer | Retain through stabilization, extract later |
| Fixtures | Too synthetic and insufficiently sourced | Replace with a provenance-aware corpus |
| External-tool runner tests | Nondeterministic in ordinary CI | Split into hermetic and pinned/opt-in lanes |
| Packaging | Incomplete | Finish after protocol stabilization |

### 3.1 Initial upstream audit targets

The first contract snapshots should be based on the official sources audited on 2026-07-12:

| Harness | Initial audit target | Approximate current event inventory | Official source |
| --- | --- | ---: | --- |
| Claude Code | Current documentation corresponding to the audited 2.1.x generation | 30 | [Claude Code hooks reference](https://code.claude.com/docs/en/hooks) |
| Codex | Documentation snapshot retrieved 2026-07-12 | 10 | [Codex hooks documentation](https://learn.chatgpt.com/docs/hooks) |
| Gemini CLI | v0.50.0-era reference | 11 | [Gemini CLI hooks reference](https://geminicli.com/docs/hooks/reference/) |
| Antigravity | 1.1.1-era reference | 5 | [Antigravity hooks reference](https://antigravity.google/docs/hooks) |

The counts are planning estimates, not hard-coded library constants. Phase 1 must enumerate the exact events and handler forms from pinned sources. Mutable documentation must receive a dated repository snapshot ID.

Gemini CLI and Antigravity remain separate supported harnesses. Google's transition messaging is product context, not evidence that their hook wire contracts are interchangeable. See [Google's transition announcement](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/).

## 4. Product goals and non-goals

### 4.1 Required goals

The stabilized library must:

- provide a native module for every supported harness;
- provide explicit, documented Rust input types for every supported event;
- preserve unknown fields and open payloads where forward compatibility requires it;
- provide event-specific, handler-specific output types for every binding the
  release target declares implemented;
- emit exact stdout, stderr, body, and exit-status behavior;
- make invalid event/output combinations impossible through normal typed APIs;
- make audience-specific messages, rewrites, denials, stops, approvals, and no-op behavior distinct where the harness distinguishes them;
- allow a concrete single-event hook to parse and run without detecting either harness or event;
- require the harness explicitly for dynamic or common execution;
- offer best-effort invocation inspection as an opt-in capability;
- accept an optional event hint during dynamic event resolution and reject demonstrable contradictions;
- provide lossless aligned-event wrapper enums with native arms;
- allow shared business logic followed by explicit harness-native lowering;
- make the post-tool-use runner clean, correct, performant, and maintainable using public library APIs; and
- expose support and verification status honestly.

### 4.2 Non-goals for the stabilization release

The stabilization work should not:

- define a lowest-common-denominator hook protocol;
- make cross-harness detection a prerequisite for execution;
- force concrete event-specific binaries to supply redundant event hints;
- claim compatibility between harnesses merely because one copied another;
- generate the public Rust API directly from JSON Schema before that approach proves beneficial;
- implement inventoried non-command bindings in Rust/runtime during this
  iteration;
- eliminate reasonable harness-specific matches from concrete hook lowering;
- put formatter/linter policy or recursive file-discovery heuristics in core/common crates;
- redesign Pkl configuration while core protocol work is underway;
- extract the post-tool-use runner before released APIs have survived its migration; or
- preserve a known-invalid `0.1` API merely to avoid a breaking pre-release change.

## 5. Controlling design principles

### 5.1 The contract inventory precedes the Rust model

No event should be revised from memory or inferred from another harness. A supported event must first have a catalog entry, provenance, request description, response descriptions, and fixtures. The catalog is a reviewed protocol ledger and the repository's presumptive source of truth.

### 5.2 Native fidelity is primary

Each harness has a distinct public identity even when it inherits or copies another protocol. Sharing internal parser pieces is acceptable; silently treating one harness as another is not.

### 5.3 Harness selection is explicit in real execution

A typed event implies its harness through its Rust type. A dynamic or common runtime must receive a harness from the caller. The runtime must never choose a production protocol solely by guessing from JSON.

### 5.4 Event knowledge belongs at the narrowest useful layer

A concrete `PostToolUse` executable already knows its event and needs no event hint. A selected-harness dynamic dispatcher may resolve an event from an authoritative discriminator or shape, and may accept a hint. A detector may inspect both harness and event, but it cannot execute the result.

### 5.5 Outputs are event and transport contracts

Empty output, JSON, plain text, HTTP bodies, stderr messages, and exit statuses are not interchangeable forms of one envelope. Output types must encode which combinations are valid for the selected event and handler kind.

### 5.6 Common wrappers are lossless

An aligned input or output enum contains full native values. Convenience accessors expose only facts that genuinely align, and absence remains explicit. Native-only capabilities always remain reachable.

### 5.7 Semantic intention helpers must earn stability

A common intention such as “manual intervention required” may be useful, but it must not become the foundation before its meaning and lowering are proven across harnesses. The first implementation should live in the concrete hook when uncertainty remains.

### 5.8 The runner is a north-star consumer, not an architecture owner

The runner must compile and retain a hermetic smoke test throughout remediation. Its successful final migration is a release condition. Its domain policy and path heuristics do not move upstream merely to reduce its local code.

### 5.9 Incorrect compatibility is worse than deliberate breakage

Obsolete builders, fictional version selectors, and permissive envelopes should be removed or replaced. If a temporary migration shim is necessary for in-repository consumers, keep it private or clearly deprecated and give it a removal milestone.

## 6. Target architecture

The logical dependency direction is:

    versioned contract catalog
                |
                v
           hookkit-core
                |
                v
       native harness crates
       |       |       |       |
     claude   codex   gemini   antigravity
        \       |       |       /
                v
          hookkit-common
                |
                v
          hookkit-runtime
                |
                v
       examples and consumers
                |
                v
      post-tool-use runner + Pkl

The catalog is logically upstream but need not be a Rust dependency. Native crates depend only on `hookkit-core`. `hookkit-common` depends on native crates. `hookkit-runtime` may depend on native/common crates to supply dispatch adapters. Pkl and runner crates remain downstream and must never become dependencies of core, native, common, or runtime crates.

The target workspace adds:

- `crates/hookkit-antigravity`;
- a non-published catalog tool such as `xtask`;
- optionally, a non-published contract conformance test crate; and
- `planning/decisions/` for durable architecture decisions.

## 7. Contract catalog specification

### 7.1 Purpose and authority

Add a top-level `contracts/` tree and treat it as a versioned protocol ledger. It records:

- the shape of JSON inputs and outputs;
- non-JSON input/output types;
- exact process and HTTP behavior;
- event and handler identity;
- source provenance;
- confidence and verification;
- known conflicts or unknowns; and
- implementation/support coverage.

JSON Schema alone is insufficient. It cannot state that zero stdout bytes differ from JSON `null`, that stderr is the denial-message channel, or that a successful `WorktreeCreate` command returns an absolute path as plain text. JSON values use JSON Schema; transport and execution semantics use validated YAML metadata.

The catalog is authoritative for this repository's interpretation of the upstream contracts. Official schemas and source remain stronger upstream evidence. When documentation, published schema, source types, and observed behavior conflict, the catalog records the conflict and the chosen compatibility policy rather than silently merging them.

Initially, schemas are test oracles and documentation. Rust-to-schema or schema-to-Rust code generation is deferred until a later ADR demonstrates that it preserves a deliberate, ergonomic, forward-compatible public API.

### 7.2 Repository layout

Use this shape as the Phase 1 starting point:

    contracts/
      README.md
      registry.yaml
      status/
        stabilization-v1.yaml
        implementation/
          claude-code.yaml
          codex.yaml
          gemini-cli.yaml
          antigravity.yaml
        observations/
      meta/
        v1/
          registry.schema.json
          harness.schema.json
          snapshot.schema.json
          event-contract.schema.json
          process-case.schema.json
      shared/
        v1/
          schemas/
            json-object.schema.json
            nonempty-string.schema.json
            absolute-path.schema.json
      vendor/
        <harness>/<upstream-version-or-commit>/
          source.yaml
          <verbatim permitted upstream files>
      harnesses/
        claude-code/
          harness.yaml
          snapshots/
            docs-2026-07-12-r1/
              snapshot.yaml
              sources.yaml
              bindings/
              events/
                worktree-create/
                  contract.yaml
                  input.schema.json
                  output.http.schema.json
                  fixtures/
                    manifest.yaml
                    input/
                    output/
                    invalid/
                    cases/
        codex/
        gemini-cli/
        antigravity/

Every harness/event pair gets a `contract.yaml`, even if its input or output schema is a one-line `$ref` wrapper around an identical shared component. That gives the repository the desired “one input and output contract per event per harness” discoverability without copying common definitions.

Do not use mutable `current` symlinks. `registry.yaml` points to the selected current snapshot for each harness.

Snapshot directories contain upstream contract evidence and interpretation only.
Mutable implementation coverage, release targets, and live observations live under
`contracts/status/` and refer back to immutable contract IDs. Implementing an event
or performing another live verification must not mutate a frozen protocol snapshot.
The release target is machine-readable and states, per event and binding, whether
the release promises catalog-only coverage, native input/output types, a command
runtime, another adapter, and/or live verification.

### 7.3 Independent version axes

Keep three versions separate:

1. `format_version`: the metadata language used by the catalog;
2. `snapshot_id`: the audited upstream contract snapshot; and
3. Rust crate/API semver.

A snapshot begins as `draft` and becomes `frozen` when its catalog review and
minimum fixtures pass. Freezing creates a deterministic content manifest and
checksum. CI rejects any mutation of a frozen snapshot; a correction creates a new
`rN` snapshot and records `supersedes`. Use an upstream release or commit when one
exists, such as `0.50.0-r1` or `commit-abc123-r1`. For mutable documentation, use a
retrieval snapshot such as `docs-2026-07-12-r1`.

Use stable local schema identifiers, for example
`urn:agent-hook-kit:contracts:claude-code:docs-2026-07-12-r1:worktree-create:input`.

The validator builds an offline URI-to-file registry. Network fetching is
forbidden in normal CI. URI-shaped `$id` and `$ref` values from byte-for-byte
vendored schemas are permitted only when the registry maps them to an approved
local vendor file.

### 7.4 Event metadata

Each event contract must record:

- metadata format version;
- stable local event ID;
- harness and snapshot;
- exact upstream wire event name and aliases;
- local Rust key;
- lifecycle/alignment category, if any;
- deprecation or replacement status;
- identification characteristics;
- discriminator path/value, when present;
- events with overlapping shapes;
- input content kind and schema;
- every documented handler binding;
- every documented outcome for each binding;
- for HTTP bindings, method, request and response content type, relevant
  protocol headers, status-code semantics, and body framing;
- stdout, stderr, body, and exit/status semantics;
- semantic constraints JSON Schema cannot express;
- source references;
- origin, confidence, and verification for each claim; and
- uncertainties and documentation/runtime discrepancies.

An illustrative `WorktreeCreate` entry should be expressible in this form:

    format_version: 1
    id: claude-code/docs-2026-07-12-r1/WorktreeCreate
    harness: claude-code
    snapshot: docs-2026-07-12-r1

    event:
      wire_name: WorktreeCreate
      rust_key: worktree_create
      category: workspace
      identification:
        inferability: definitive
        discriminator:
          json_pointer: /hook_event_name
          const: WorktreeCreate

    schemas:
      input:
        file: input.schema.json
        origin: derived
        sources: [claude-hooks-worktree-create]
        assurance:
          confidence: high
          verification: source-reviewed

    bindings:
      command:
        kind: process
        request:
          channel: stdin
          framing: single-document-at-eof
          content:
            kind: json
            schema: input
        outcomes:
          - id: created
            meaning: use the emitted path as the created worktree
            exit: { exact: 0 }
            stdout:
              presence: required
              role: protocol-value
              content:
                kind: text
                encoding: utf-8
                semantic_format: absolute-path
                trailing_newline: allowed
            stderr:
              presence: optional
              role: diagnostics
              content: { kind: text, encoding: utf-8 }
            sources: [claude-hooks-worktree-create]
            assurance:
              confidence: high
              verification: source-reviewed

The final metadata schema should use tagged unions, reject unknown metadata keys, and reject duplicate YAML keys. YAML anchors, merge keys, and implicit surprising scalar coercions should be disallowed or linted out.

### 7.5 Content and channel kinds

The first metadata version must support:

- `empty`: exactly zero bytes, not a newline, `{}`, or `null`;
- `json`: a JSON document with a schema reference;
- `text`: encoding, presence, length/newline rules, and optional semantic format;
- `opaque`: an exceptional binary or unspecified representation with a required rationale and low-confidence marker.

Text semantic formats may include `absolute-path`, but ordinary JSON Schema should not fake cross-platform path validation with a POSIX-only regular expression. The catalog checker may implement semantic validation separately.

Process metadata must model:

- exact, set, or range exit-code selectors;
- stdout and stderr independently;
- required, optional, or forbidden stream presence;
- request framing and output newline rules;
- channel role, such as protocol value, user message, agent context, or diagnostics;
- semantic effect, such as continue, deny, ask, rewrite, stop, fail, or provide value; and
- unmatched exit-code behavior.

There is no universal “exit 2 means deny” rule. It is defined per harness, event, and sometimes handler binding.

The catalog must distinguish all of:

- zero-byte stdout;
- JSON `{}`;
- JSON `null`;
- a plain-text value;
- stderr-only blocking;
- a hook process failure;
- a protocol-level negative decision emitted with exit status zero; and
- command versus HTTP response forms.

Catalog all known handler kinds. For HTTP, metadata v1 must be capable of recording
method, content types, relevant headers, status selection, and body framing; an
entry that has only a body schema is explicitly `transport-incomplete` and cannot
claim full binding coverage. The first runtime release may implement only
executable command hooks, but the support report must distinguish “inventoried”
from “runtime adapter implemented.”

### 7.6 JSON Schema policy

Use JSON Schema Draft 2020-12.

- Every JSON input has an event-root schema.
- Every distinct JSON response representation has an event/binding-root schema.
- Root schemas may be small `$ref` wrappers.
- Share only wire-identical structures.
- Harness-specific common envelopes stay within that harness snapshot.
- Conceptually similar cross-harness records remain separate.
- Every object explicitly declares its extra-property policy.
- Use `unevaluatedProperties: false` only when the upstream contract is genuinely closed.
- Use `oneOf` for exclusive shapes and include invalid combination fixtures.
- Preserve native casing and names; compatibility aliases belong in an explicit compatibility policy, not the canonical schema.
- Do not use schema reuse as evidence of semantic compatibility.

Normative wire acceptance and Rust forward compatibility are related but distinct. A Rust type may preserve unknown members for diagnostics and future compatibility even when the catalog does not claim the upstream accepts arbitrary fields. Record both policies explicitly.

For a future harness described as “Claude-compatible with deviations,” create a distinct harness snapshot and event roots. Record `wire_heritage` or `derived_from` metadata, reuse exact components, and enumerate deviations. Never inherit future Claude changes automatically or silently fall back to the Claude parser.

### 7.7 Provenance, confidence, and vendoring

`sources.yaml` contains reusable source records with:

- source kind and authority;
- URL and stable section/locator;
- retrieval date;
- upstream version, tag, or commit;
- local vendored path, if any;
- content checksum, if useful and permitted; and
- license information for vendored material.

Every schema and protocol outcome separately records:

- origin: `vendored`, `derived`, or `authored`;
- source references;
- confidence: `high`, `medium`, or `low`;
- verification: `unverified`, `fixture-validated`, `source-reviewed`, or `live-observed`; and
- optional harness version, platform, and date for live observations.

Origin, confidence, and verification are independent. Official prose can be ambiguous. A live observation can be strong evidence for one version/platform without proving universal behavior.

Assurance inside a frozen snapshot records evidence available at freeze time.
Later live observations are appended to `contracts/status/observations/`. Create a
new protocol snapshot only when that evidence changes the interpreted contract,
not merely to update a verification date.

When upstream publishes a usable schema:

1. vendor it byte-for-byte when licensing permits;
2. record URL, revision, retrieval date, checksum, and license;
3. never patch the vendored copy;
4. reference it through a local event root where possible; and
5. create a separately documented derived schema for corrections or normalization.

When only source types exist, reference a pinned revision and derive a schema. When
only mutable documentation/examples exist, require one of:

- a pinned source revision;
- a permitted local capture; or
- a documented retrieval recipe plus a deterministic normalized-content hash.

If none is possible, mark the source unreproducible, set confidence to low, and
explain the limitation. Refresh commands may discover changes, but must never
silently overwrite the current snapshot.

### 7.8 Fixtures

Before a Phase 1 event contract can freeze, it must have:

- a minimal valid input;
- a representative input containing documented optional fields;
- valid JSON output fixtures for every distinct output form;
- text fixtures for every text outcome;
- negative fixtures for missing required fields, wrong discriminators, and mutually exclusive output traps; and
- process cases that assert input, selected outcome, exit code, stream content,
  and framing.

`fixtures/manifest.yaml` (or validated per-fixture sidecars if the format spike
chooses them) must enumerate every fixture and reject orphan files. Each entry
records origin, sources, expected schema validity or protocol outcome,
sanitization, live-verification status, and any expected failure pointer/keyword.
Every fixture declares its origin:

- copied from official material;
- sanitized live capture;
- synthesized from official documentation/source; or
- regression fixture.

Synthetic fixtures must not masquerade as live captures, and one Claude-shaped `Write` payload must not stand in for multiple harnesses. Sanitization may replace IDs and paths but must not alter structural shape.

Each negative fixture should contain one intended defect and, where practical,
assert a stable JSON instance pointer and validation keyword. This avoids
validator-order-dependent failures.

Exact empty/text/opaque stream fixtures should include checksums so an editor
cannot silently add a trailing newline. Store expected bytes as base64/hex in case
metadata or mark those fixture paths as non-normalized/binary in `.gitattributes`
so Git newline conversion cannot alter them. JSON output is normally checked
structurally and against its schema, with framing and newline behavior checked
separately; require byte-for-byte JSON only if a protocol requires canonical
serialization. Runner-specific tool-output fixtures remain with the runner; native
hook payload fixtures move into the contract snapshot over time. A documented,
machine-readable gap is the only exception to the minimum fixture set.

### 7.9 Detection metadata

The catalog supports the optional inference capability without becoming a general inference rule engine. Each event records one of:

- `definitive`: an authoritative discriminator proves the event;
- `shape-based`: recognizable but not guaranteed unique;
- `ambiguous`: overlaps listed events; or
- `impossible`: identical shapes require external event knowledge.

Without a hint, ambiguous matches remain ambiguous. With a hint, the resolver
validates through that event's exact native parser/validator. It accepts
ambiguous-but-compatible input as the hinted event, but reports a mismatch when a
catalog-declared sound predicate demonstrably proves another event. General schema
acceptance alone is weak evidence unless the event contract explicitly declares it
identifying.

### 7.10 Catalog tooling

Add a small non-published workspace tool with:

    cargo xtask contracts check
    cargo xtask contracts check --snapshot current
    cargo xtask contracts report
    cargo xtask contracts diff <old> <new>
    cargo xtask contracts verify-vendor

`contracts check` must:

1. validate YAML against the metadata meta-schemas;
2. reject duplicate IDs, duplicate/unknown YAML keys, missing files, and missing sources;
3. compile every JSON Schema with offline `$ref` resolution;
4. reject unresolved references and all network fetching; URI-shaped references
   are allowed only when the local registry resolves them inside approved roots;
5. validate all positive fixtures;
6. prove all negative fixtures fail, preferably at the expected pointer/keyword;
7. verify exact non-JSON stream fixture checksums and JSON framing assertions;
8. ensure every snapshot-index event has a contract;
9. ensure every JSON request/response references a schema; and
10. ensure every non-JSON response has explicit channel, content, and process metadata.

The resolver must sandbox file references to `contracts/` and approved vendor
roots, reject path traversal, and never read arbitrary repository or host files.

`contracts report` generates separate coverage columns:

- inventoried;
- input schema complete;
- output/process contract complete;
- native input modeled;
- native output modeled;
- command runtime supported;
- other handler runtime supported;
- hermetic conformance tested; and
- live verified.

The report, not a handwritten README table, becomes the source of public support claims.

Use explicit support levels rather than compressing columns into one boolean:

- `catalog-only`;
- `native-source-reviewed`;
- `command-runtime-beta`;
- `command-runtime-stable`;
- `other-runtime-beta/stable`; and
- `unsupported` with a reason.

Verification remains an independent field so an event cannot become “stable” only
because a type exists.

Rust coverage must come from a machine-enumerable registry, not source-name
conventions or a second handwritten matrix. Each native harness crate exposes
static event descriptors containing exact event ID, target snapshot/contract IDs,
implemented binding adapters, and conformance case IDs. The report combines these
registries with the mutable `contracts/status/` target and observation overlays.
CI rejects registry entries that have no catalog contract and target entries that
claim implementation not present in the registry.

## 8. Target Rust API and runtime semantics

The examples in this section define responsibilities and type relationships. Phase 2 may improve names and module boundaries.

To avoid a dependency cycle, `hookkit-core` owns protocol identity types, catalog
references/descriptors, `RawInvocation`, `RuntimeContext`, `EventSpec`,
`HarnessSpec`, and the encoded `ProcessEmission` representation. Native crates
implement the traits while depending only on core. `hookkit-runtime` owns stdin,
stdout, stderr, process exit, and dynamic dispatch entry points and depends on the
native/common crates. A future leaf `hookkit-protocol` crate is an acceptable
alternative only if the Phase 2 ADR updates this dependency rule consistently.

### 8.1 Exact identity types

`hookkit-core` should distinguish exact protocol identity from aligned concepts:

    pub struct HarnessId(&'static str);

    #[non_exhaustive]
    pub enum BuiltinHarness {
        ClaudeCode,
        Codex,
        GeminiCli,
        Antigravity,
    }

    pub struct EventId {
        pub harness: HarnessId,
        pub name: &'static str,
    }

    pub struct SnapshotId(&'static str);
    pub struct ContractId(&'static str);

    #[non_exhaustive]
    pub enum AlignedEventKind {
        SessionStart,
        PreToolUse,
        PostToolUse,
        Stop,
        // Only genuinely aligned concepts belong here.
    }

`EventId` is used for protocol identity, parsing, validation, errors, and output
matching. `SnapshotId` and `ContractId` tie the compiled implementation to an
immutable interpretation of that protocol. `AlignedEventKind` is only a
convenience for common wrappers and capability queries; it never replaces exact
identity.

### 8.2 Native event specification

Each native event associates its input and valid command output:

    pub trait EventSpec {
        type Harness: HarnessSpec;
        type Input;
        type CommandOutput;

        const ID: EventId;
        const CONTRACT: ContractId;
        const INPUT_SCHEMA: ContractId;
        const COMMAND_BINDING: Option<ContractId>;

        fn decode(raw: &RawInvocation) -> Result<Self::Input, DecodeError>;
        fn encode_command(
            output: Self::CommandOutput,
        ) -> Result<ProcessEmission, EncodeError>;
    }

    pub trait HarnessSpec {
        const ID: HarnessId;
        const SNAPSHOT: SnapshotId;
        const DIALECT_OF: Option<HarnessId>;

        type AnyInput;
        type AnyCommandOutput;
        type EventSelector;

        fn resolve_invocation(
            raw: &RawInvocation,
            hint: Option<Self::EventSelector>,
        ) -> Result<ResolvedInvocation<Self::AnyInput>, ResolveError>;

        fn event_descriptors() -> &'static [EventDescriptor];
    }

The exact associated-type design must include a harness-scoped event selector; a
Codex event hint passed to a Claude resolver fails immediately as
`HintHarnessMismatch` before payload analysis. Handler-specific associated types
or traits may replace `CommandOutput` when HTTP or other runtime adapters are
implemented. The ordinary API must still make an event's valid output finite and
explicit.

Native modules should be event-oriented:

    hookkit_claude::events::post_tool_use::{
        Event,
        Input,
        CommandOutput,
        StructuredResponse,
    }

Event-wide dynamic enums remain useful for selected-harness dispatch, but their output arm must match the triggering input before bytes are emitted.

### 8.3 Three execution modes

Keep three concerns separate.

#### A. Exact typed event execution

For a concrete hook that knows both harness and event:

    run_event::<claude::events::post_tool_use::Event>(handler)

The Rust type supplies harness and event. There is no detection and no hint.

#### B. Selected-harness dynamic execution

For a binary that handles multiple events within one caller-selected harness:

    run_harness::<ClaudeCode>(
        optional_claude_event_hint,
        handler,
    )

This compile-time generic form uses that harness's `AnyInput`,
`AnyCommandOutput`, and event selector. A binary that receives the harness at
runtime uses a deliberately separate built-in dispatcher:

    dispatch_builtin_harness(
        selected_harness,
        optional_scoped_event_hint,
        handler_using_builtin_input_and_output_enums,
    )

The second form needs explicit umbrella input/output enums and verifies both
harness and event arms. Do not hide these two type models behind one ambiguous
`run_harness` signature.

In both forms the harness is mandatory. The resolver may use a catalog-declared
authoritative event discriminator, another sound identification predicate, or the
optional hint. Some ambiguous protocols may require a hint in this mode.

#### C. Aligned multi-harness execution

For a `PostToolUse` hook with shared business logic:

    run_aligned_event::<PostToolUse>(
        selected_harness,
        handler,
    )

The event is known statically; the harness is mandatory dynamically. The input and output wrappers retain native arms.

There should be no ordinary `run_auto` API that guesses a harness and immediately executes.

### 8.4 Invocation resolution and event hints

The selected-harness resolver accepts an optional event hint with these semantics:

1. If the hint names another harness, return `HintHarnessMismatch` before
   inspecting the payload.
2. If an authoritative discriminator conflicts with the hint, return
   `EventHintMismatch`.
3. If there is no discriminator and the hinted event's native parser/validator
   accepts the payload, select the hint.
4. If the hinted event rejects the payload and another event is uniquely
   established by catalog-declared sound evidence, return `EventHintMismatch`
   with the observed candidate.
5. If the hinted event rejects the payload but no other event is established,
   return `InvalidInputForHint`.
6. Without a hint, use an authoritative discriminator when present.
7. Without a discriminator, select an event only when its catalog-declared
   identification predicate is classified as sound and uniquely matches.
8. If only weak schema/shape candidates or multiple sound candidates remain,
   return `AmbiguousEvent` with candidates and require a hint.
9. If none match, return `UnrecognizedEvent` with evidence.

Resolution should retain provenance such as discriminator, schema, or hint so diagnostics and conformance tests can explain the choice.

A hint constrains resolution; it does not overwrite contradictory evidence. Mere
success against one open JSON Schema is weak evidence unless the catalog
explicitly declares the distinguishing predicate sound. A concrete typed event
parser follows the equivalent validation rule but does not call the general
inference engine.

### 8.5 Best-effort detector

Offer a separate, non-executing inspection API:

    let report = Detector::builtins().inspect(
        &raw,
        DetectionConstraints {
            harness: optional_harness_constraint,
            event: optional_event_hint,
        },
    );

The result should include:

- zero, one, or multiple harness/event candidates;
- evidence and its strength;
- ambiguity and overlapping schemas;
- mismatched supplied constraints; and
- raw validation failures suitable for diagnostics.

Both constraints are optional in inspection mode. An event hint follows the same
contradiction rules as selected-harness resolution; a harness constraint likewise
narrows candidates but cannot relabel evidence that definitively identifies another
harness. The detector is useful for debugging, fixture classification, migration
tools, and user-facing “what payload is this?” commands. It must not be the hidden
first step of typed or selected-harness execution.

Production hooks should not compile or scan the full catalog on every invocation.
Phase 2 should generate or hand-maintain lightweight native identification
descriptors from reviewed catalog predicates, then run the exact native parser
after resolution. Full JSON Schema evaluation belongs in `contracts check`,
conformance tests, and optionally a detector feature intended for diagnostics.
Parity tests must prove that lightweight predicates correspond to their catalog
entries; agents must not duplicate ad hoc casing probes in the runtime.

### 8.6 Event-safe outputs

Output names and constructors should encode the event:

- `ClaudePostToolUseCommandOutput`;
- `ClaudeSessionStartCommandOutput`;
- `ClaudeWorktreeCreateCommandOutput`; and
- similarly exact types for each harness.

The type system should distinguish:

- no opinion/no protocol response;
- explicit approval;
- ask/defer;
- denial/block;
- rewrite;
- stop/continue;
- user-visible message;
- agent-visible context;
- process failure; and
- protocol outcome emitted with a successful process exit.

Do not conflate handler failure with a protocol denial. Do not make “allow” synonymous with “no opinion” where the harness distinguishes them.

The event discriminator in a structured output should be inserted or validated internally. A Claude `SessionStart` handler must not be able to accidentally emit `hookEventName: PostToolUse` through a shared builder.

Unchecked raw emission may exist for forward compatibility, but it must be explicit, visibly unsafe/unchecked, and excluded from event-safe support guarantees.

### 8.7 Internal process emission

Use a small internal runtime representation:

    pub enum StdoutPayload {
        Empty,
        Json(serde_json::Value),
        Text(String),
        Bytes(Vec<u8>),
    }

    pub struct ProcessEmission {
        contract: ContractId,
        stdout: StdoutPayload,
        stderr: Vec<u8>,
        exit_code: u8,
    }

The exact types may use owned bytes or serializers for efficiency. Fields remain
private. Protocol-author constructors require an exact contract/binding descriptor
and validate content/channel/status constraints; typed handler APIs never accept
`ProcessEmission` directly. If raw construction must be public because native
crates are separate packages, it is explicitly protocol-author/unchecked surface,
not a convenient application builder. The important constraint is that this is a
wire-level target. Native event outputs lower into it only after validation.

Runtime responsibilities are:

- read stdin without contaminating stdout;
- retain raw input for diagnostics and forward compatibility;
- parse exactly once where practical;
- invoke the handler;
- verify dynamic input/output arm agreement;
- lower through the native event contract;
- validate stream/exit constraints;
- write stdout and stderr exactly once;
- return the exact exit status; and
- avoid cloning large payloads only to remember identity.

Exact-byte tests are mandatory for empty, text, opaque/binary output, stream
choice, and newline/framing behavior. JSON is normally asserted structurally and
against its schema while its framing/newline behavior is exact; byte-for-byte JSON
is required only where the protocol defines canonical bytes.

### 8.8 Runtime context, unknown events, and diagnostics

Handlers receive a `RuntimeContext` containing:

- exact harness, snapshot, event, and contract identity;
- resolution provenance, or `typed-static` for exact event execution;
- raw input bytes and parsed JSON access without reparsing;
- zero or more exact workspace roots;
- optional typed session, conversation, transcript, tool-call, and artifact
  identifiers only when the native contract supplies them; and
- a configured diagnostics sink.

The runtime must not default a missing workspace to `.`, guess fields through
camelCase/snake_case probes, or recursively mine arbitrary JSON into context. Any
consumer heuristic is explicit and downstream.

An unknown event remains inspectable through the returned error/raw invocation and
the detector. Selected-harness execution rejects it before invoking a handler,
because there is no known safe output contract. Passing it through or emitting a
no-op requires the explicit unchecked API and cannot be presented as supported.

Protocol stderr belongs exclusively to the selected native emission. Runtime
tracing and handler diagnostics use the configured sink (for example a callback,
file, or disabled sink) and are never written automatically to stdout or merged
into stderr. A binding may explicitly allow diagnostic merging, but the native
encoder performs that merge under its contract. Documentation must warn that
uncontrolled `println!`/`eprintln!` in handler code can violate the hook protocol.

### 8.9 Lossless common wrappers

Aligned inputs and outputs contain native values:

    #[non_exhaustive]
    pub enum PostToolUseInput {
        ClaudeCode(hookkit_claude::PostToolUseInput),
        Codex(hookkit_codex::PostToolUseInput),
        GeminiCli(hookkit_gemini::AfterToolInput),
        Antigravity(hookkit_antigravity::PostToolUseInput),
    }

    #[non_exhaustive]
    pub enum PostToolUseOutput {
        ClaudeCode(hookkit_claude::PostToolUseCommandOutput),
        Codex(hookkit_codex::PostToolUseCommandOutput),
        GeminiCli(hookkit_gemini::AfterToolCommandOutput),
        Antigravity(hookkit_antigravity::PostToolUseCommandOutput),
    }

Useful common APIs include:

- `harness()` and exact `event_id()`;
- native-arm accessors or pattern matching;
- raw payload access;
- workspace roots as a collection rather than forcing one `cwd`;
- optional tool name/call/result only where the native event supplies them; and
- exact documented path fields.

Recursive “find a likely modified file somewhere in this JSON” heuristics belong in concrete consumers, not `hookkit-common`.

The runtime verifies that a returned native output arm matches the input arm. Unsupported semantics fail explicitly; they never degrade silently.

Built-in aligned enums are `#[non_exhaustive]` so adding a future harness does not
promise exhaustive matching across crate versions. If downstream extensibility
beyond built-ins becomes a requirement, use a registration mechanism or
consumer-owned wrapper rather than an ever-growing closed enum.

### 8.10 Common semantic intentions

Do not make a stable, library-wide semantic result a prerequisite for the first migration.

The post-tool runner should first define a local domain result, for example:

    enum ToolCheckOutcome {
        Clean,
        AutoFixed { ... },
        ManualActionRequired { ... },
        ToolFailure { ... },
    }

It then lowers that result explicitly into each available native output. This tests whether a reusable intention exists without prematurely committing the library.

A common semantic intention may be promoted only if:

- at least two harnesses implement materially the same concept;
- its name has the same meaning for each;
- lowering is total or reports a typed unsupported error;
- no audience, rewrite, stop, permission, or message field is silently discarded;
- it benefits more than the post-tool runner alone; and
- no-op, approval, denial, ask, defer, rewrite, and stop remain distinct.

If these conditions are not met, keep the intention hook-local, remove it, or place it behind an explicitly experimental feature/module. Record the decision in an ADR after the runner migration supplies evidence.

### 8.11 Protocol dialects

Future “compatible” harnesses receive:

- a distinct `HarnessId`;
- a native module/crate;
- a catalog snapshot;
- optional `DIALECT_OF`/`wire_heritage` metadata; and
- explicit deviations.

Exact schemas and internal parsers may be reused when truly identical. Public values preserve the actual harness identity. Parsing must not silently fall back to the parent protocol when a deviation fails.

## 9. Roadmap overview

| Phase | Outcome | Depends on | Parallelism |
| --- | --- | --- | --- |
| 0. Baseline and decisions | Deterministic baseline, scoped support promise, ADR set | None | Test/baseline and ADR drafting can overlap |
| 1. Contract catalog | Validated current snapshots, schemas, fixtures, reports | Phase 0 format decision | Four harness inventories parallel after the format spike |
| 2. Typed protocol foundation | Event-safe API, exact process emission, representative vertical slices | Relevant Phase 1 spike contracts | Core/API and conformance work can partly overlap |
| 3. Native harness remediation | Complete faithful native crates for four target harnesses | Phase 2 plus that harness's catalog | Claude, Codex, Gemini, Antigravity parallel |
| 4. Common model and dynamic runtime | Lossless aligned wrappers and explicit-harness dispatch | Relevant aligned native event slices stable | Input/output wrappers can split by aligned event and overlap remaining Phase 3 work |
| 5. Runner north-star migration | Real consumer works cleanly through public APIs | Phase 4 PostToolUse path | Domain/test work can start before final lowering |
| 6. Packaging, documentation, release | Consumable, verified library release | Phases 3–5 | Package and docs work can overlap late |
| 7. Extraction and maintenance | Optional downstream runner and durable drift loop | Stable released APIs | Post-release |

Phases are gates, not necessarily monolithic branches. A harness may begin Phase 3 when its Phase 1 catalog is complete and the relevant Phase 2 foundation has landed; it does not need to wait for every other harness inventory. Likewise, Phase 4 `PostToolUse` work may begin when the required native `PostToolUse` slices are stable, before unrelated native events finish.

## 10. Detailed implementation phases

### Phase 0 — Baseline, scope, and architecture decisions

#### Objective

Create a stable point of comparison and eliminate ambiguity before protocol-facing rewrites.

#### Work package 0A: durable baseline

Record in a checked-in audit artifact:

- baseline commit and date;
- workspace crate/dependency graph;
- public API inventory;
- events currently modeled by each native crate;
- examples and runner call sites for each API;
- format, Clippy, unit, integration, documentation, package, and runner test results;
- the six currently observed full-suite failures and their external dependencies;
- package/publish gaps; and
- repository size concentration in the runner/Pkl area.

The artifact should link commands and classify every failure as:

- protocol/library defect;
- deterministic runner defect;
- external-tool compatibility;
- missing dependency/environment; or
- stale expected output.

Do not update goldens to match arbitrary local tool versions.

#### Work package 0B: architecture decision records

Create `planning/decisions/` and record at least:

1. contract catalog authority, format, snapshot/version policy;
2. explicit harness selection and optional inference;
3. typed event APIs versus dynamic dispatch;
4. event/handler-specific output types;
5. internal process emission representation;
6. native-arm common output wrappers;
7. semantic intention promotion policy;
8. Gemini CLI and Antigravity as distinct harnesses;
9. compatible/dialect harness identity and reuse policy;
10. command runtime scope versus other inventoried handlers;
11. crate/package ownership and intended composable package boundaries; and
12. runner retention and later extraction gate.

These are Phase 0 blocking policies, not a demand to decide later
evidence-dependent outcomes. For example, the semantic-intention ADR records
“runner-local first; promotion decided in Phase 5,” and the package ADR fixes
crate ownership/dependency boundaries while leaving publication automation or an
optional facade to Phase 6. Later decision records may remain explicitly scheduled
for their named phase. The ADRs should reference this plan and may settle names
that this document leaves illustrative.

#### Work package 0C: test lanes

Define and document:

1. offline contract/meta-schema checks;
2. hermetic core/native/runtime tests required for each pull request;
3. hermetic runner orchestration using fake executables;
4. pinned or opt-in real formatter/linter compatibility;
5. optional/scheduled live harness conformance; and
6. optional/scheduled upstream drift discovery.

Ordinary pull requests should run lanes 1–3. Lanes 4–6 may be scheduled or manually triggered but must report versions and artifacts.

#### Phase 0 exit gate

- The baseline is reproducible.
- Every known failure is classified.
- Ordinary protocol tests are deterministic.
- Initial release scope and handler-kind support are explicit.
- Phase 0 blocking ADR policies are accepted; later evidence-dependent decisions
  have an owner and deadline.
- The runner still compiles and has a small hermetic smoke path.

### Phase 1 — Contract inventory and fixture corpus

#### Objective

Establish reviewed protocol truth before rewriting models.

#### Work package 1A: format spike

Implement metadata meta-schemas, the offline URN registry, YAML restrictions, and `cargo xtask contracts check`. Prove the format against four deliberately difficult vertical cases:

- Claude `WorktreeCreate` command plain-text output versus HTTP JSON output;
- current Codex `PreToolUse` control behavior;
- Gemini CLI `BeforeToolSelection` output;
- Antigravity `PreInvocation`/`PostInvocation` input ambiguity.

These cases must exercise JSON, non-JSON, handler differences, event-specific control, and hint metadata. Revise the format before bulk population; after approval, increment `format_version` for incompatible changes.

#### Work package 1B: snapshot/source indexes

For each target harness:

- create `harness.yaml` and one current immutable snapshot;
- pin release/tag/commit where possible;
- otherwise create a dated docs snapshot;
- enumerate every documented event and handler binding;
- create reusable source records;
- record deprecations, aliases, and product transitions; and
- mark unresolved questions explicitly.

This work establishes inventory coverage before full schemas exist.

#### Work package 1C: per-harness input contracts

Each harness can proceed in an independent branch/PR:

- author event-root input schemas;
- review exact casing, names, requiredness, and object openness;
- add minimal and representative positive fixtures;
- add discriminator, missing-field, and exclusivity negative fixtures;
- record provenance and assurance; and
- document overlap/ambiguity for resolution.

A native input rewrite for a harness may start only after its input catalog passes review.

#### Work package 1D: per-harness output and process contracts

For every event/binding:

- inventory every structured response alternative;
- model empty, JSON, text, stderr, body, and exit/status behavior;
- identify user-versus-agent channels and protocol effects;
- cover output discriminators and mutually exclusive shapes;
- add exact process cases and output fixtures;
- document no-op versus explicit approval; and
- record unsupported or undocumented outcomes rather than inventing them.

A native output/runtime rewrite may start only after the relevant output/process contracts pass review.

#### Work package 1E: catalog report and parity interface

Implement catalog-only report generation and define the machine-readable interface
that native registries will satisfy in Phase 2. At this stage the report may state
that existing Rust coverage is unknown or legacy; it must not infer conformance
that the remediated types/runtime do not yet provide.

Define the schema for temporary implementation gaps. Each entry includes exact
harness, snapshot, event, binding, failed assertion, rationale, owner, removal
phase, and optional expiry. CI must reject unmatched new failures, stale entries
whose expected failure no longer occurs, and unreviewed growth. Phase 2 adds the
Rust conformance runner that consumes this interface. The support report displays
inventory coverage separately from implementation coverage.

#### Phase 1 exit gate per harness

- Every documented event and handler is indexed.
- Every JSON request has a compiling Draft 2020-12 root schema.
- Every response has a JSON schema or explicit non-JSON descriptor.
- Process/HTTP outcomes are complete enough to implement.
- Positive, negative, and exact transport fixtures pass offline.
- All claims have sources and assurance.
- Known conflicts remain visible.
- `contracts check` and report generation pass.

### Phase 2 — Typed protocol and runtime foundation

#### Objective

Make protocol-invalid output difficult or impossible before bulk native implementation.

#### Work package 2A: core identity and raw input

Implement:

- exact harness and event identifiers;
- aligned event categories kept separate from exact identity;
- a raw invocation type retaining bytes/JSON and diagnostic context;
- structured parse/resolve/encode errors;
- dialect lineage metadata with no implicit parser fallback; and
- catalog IDs or references that native events can expose.

Avoid a global enum that must be edited for every third-party dialect if an open identifier plus built-in convenience enum serves better.

#### Work package 2B: event specification and event-safe outputs

Prototype the event-associated input/output API. Ensure:

- concrete event input and output are associated;
- handler kind is explicit;
- structured output discriminators are not caller-controlled incorrectly;
- process failures are not protocol decisions;
- no-op and approval remain distinct; and
- raw/unchecked output is visibly outside checked guarantees.

Add compile-fail or equivalent type-level tests for invalid event/output pairings.

#### Work package 2C: exact process emission

Implement internal `ProcessEmission` and a typed exact-event runner. Test:

- zero stdout bytes;
- JSON stdout with the required event discriminator;
- plain-text stdout with/without trailing newline as allowed;
- stderr-only protocol messaging;
- nonzero exit behavior;
- malformed handler output;
- handler error policy; and
- no duplicate or contaminated writes.

#### Work package 2D: representative vertical slices

Implement through catalog, native type, runtime, and exact wire-semantics test:

- Claude `SessionStart`;
- Claude command `WorktreeCreate`;
- one current Codex tool-control event;
- one Gemini output requiring the correct event-specific `hookEventName`; and
- one discriminator-free Antigravity event. This work package creates the minimal
  `hookkit-antigravity` crate and first event; Phase 3D completes the crate.

The known Claude example bug—serializing a `SessionStart` result with a `PostToolUse` event name—must become impossible through the normal API.

#### Work package 2E: resolution and detector API

Implement selected-harness event resolution, optional hint behavior, inference provenance, and the non-executing cross-harness detector. Include tests for:

- definitive discriminator;
- matching hint;
- discriminator/hint contradiction;
- invalid input for hint;
- unique catalog-declared sound predicate;
- a weak single schema/shape candidate that still requires a hint;
- ambiguous no-hint result;
- Antigravity ambiguity resolved by a hint;
- zero candidates; and
- multiple harness candidates from compatibility-shaped inputs.

Do not add automatic execution of detector results.

#### Work package 2F: Rust conformance runner

Add the non-published conformance runner against the Phase 1 registry interface.
It must prove, for implemented vertical slices:

- catalog-valid inputs parse into the expected native event;
- required fields reach typed Rust fields rather than only an `extra` map;
- unknown fields are retained according to policy;
- typed outputs serialize into an allowed catalog outcome;
- runtime cases emit correct stream content, framing, and status;
- outputs cannot cross events; and
- resolver/detector behavior matches catalog identification metadata.

Activate only exact predeclared gap entries. New or changed failures must fail CI;
a gap whose assertion starts passing must also fail until the obsolete entry is
removed.

#### Phase 2 exit gate

- Invalid event/output combinations are blocked by types or checked dynamic matching.
- `WorktreeCreate` emits the documented plain text.
- Typed hooks perform no harness/event guessing.
- Dynamic hooks require an explicit harness.
- Hint contradictions and ambiguity are deterministic and tested.
- Representative emissions satisfy catalog schemas and exact transport cases.
- The runner compiles through a small temporary adapter if necessary.

### Phase 3 — Native harness remediation

#### Objective

Bring each native crate into conformance with its current catalog snapshot. The
four workstreams can run in parallel once Phase 2 and the complete event contracts
(input plus every binding/output needed by that work package) are ready.

Every event is complete only when:

1. its catalog contract is reviewed;
2. its input schema and fixtures pass;
3. Rust required/optional fields match the catalog;
4. unknown additions are retained where intended;
5. every output alternative promised by the machine-readable stabilization target
   has an event-specific type;
6. runtime tests assert exact non-JSON bytes, JSON structure/schema plus framing,
   stderr behavior, and status;
7. invalid combinations have negative/type-level coverage;
8. catalog/Rust parity is green;
9. the support report is accurate; and
10. live verification status is recorded in the mutable observation overlay.

The default stabilization scope is: catalog every documented binding, implement
native input plus command-output/runtime support where the target selects it,
and mark other handler bindings unsupported. A future iteration may reconsider
that boundary through a new explicit scope decision. Phase 3 must not fabricate
Rust support for a non-command binding merely because its body schema resembles
command output.

#### Work package 3A: Claude Code

- Re-audit all current events rather than extending the old subset opportunistically.
- Add missing common and event-specific input fields.
- Update built-in tool payloads while keeping MCP and unknown tool names/payloads open.
- Replace the universal output envelope with event-specific outputs.
- Inventory output differences by command, HTTP, prompt, agent, or other
  documented handler kinds, and implement event-specific Rust types/adapters for
  those bindings selected by the stabilization target.
- Correct `SessionStart` output and discriminator behavior.
- Correct post-tool updates, additional context, and event-specific control.
- Correct `UserPromptSubmit` suppression/control semantics.
- Model worktree command text output separately from HTTP output.
- Add current terminal, notification, stop, subagent-stop, and other audited fields/events.
- Report handler bindings that are inventoried but not executable by this library separately.

Split this work into coherent event-family PRs if necessary. Each PR must keep the native crate and runner adapter compiling.

#### Work package 3B: Codex

- Replace the obsolete five-event assumption with the current cataloged event set.
- Correct snake_case names and current field meanings.
- Remove fictional protocol-version selection that cannot be inferred from invocation input.
- Make documented required fields required in the native types.
- Preserve unknown event/tool extensions and open payloads.
- Implement exact permission, deny/block, ask, rewrite, context, stop, and no-op forms where documented.
- Pick and document a canonical emitted form when Codex accepts compatibility alternatives.
- Replace Claude-shaped synthetic `Write` fixtures with native Codex fixtures.
- Add an example using current input and output semantics.

This is a rewrite of the protocol-facing surface, even if internal helpers can be retained.

#### Work package 3C: Gemini CLI

- Keep the Gemini CLI harness identity and crate.
- Correct common and event-specific snake_case fields.
- Rewrite current model request/response shapes.
- Keep tool names and payloads open; do not encode a fictional fixed `shell` contract.
- Correct `BeforeToolSelection` input and output.
- Require the correct event-specific `hookEventName` in structured output.
- Model rewrite, deny, context, and stop behavior exactly as cataloged.
- Replace shared synthetic fixtures with current Gemini CLI fixtures and provenance.
- Add an example covering a capability that differs from Claude/Codex.

#### Work package 3D: Antigravity

- Complete the minimal `hookkit-antigravity` crate and distinct harness identity
  introduced by Phase 2D.
- Implement the five current event families from the audited snapshot.
- Model conversation/session metadata, transcript path, artifact directory, and multiple workspace roots.
- Model the absence of an authoritative event discriminator.
- Require event selection/hint in dynamic cases that remain ambiguous.
- Test matching hints, contradictions, and indistinguishable payloads.
- Do not invent a tool call, changed-file result, or other fields absent from Antigravity `PostToolUse`.
- Reuse internals only for exact inherited shapes and retain Antigravity public identity.
- Add an example that demonstrates explicit event knowledge or hint behavior.

#### Phase 3 exit gate

- `contracts/status/stabilization-v1.yaml` has no unaccounted target entry: every
  harness/event/binding has an explicit intended level (catalog-only, native
  input/output, command runtime, other adapter, and live verification), and every
  promised implementation level meets the native event definition of done.
- No known protocol-invalid convenience builder remains.
- Unknown events/fields remain inspectable where appropriate.
- Each native crate has a small current executable example.
- Protocol tests are hermetic and green.
- The generated support report distinguishes cataloged, implemented, runtime-supported, and live-verified states.

### Phase 4 — Lossless common model and dynamic runtime

#### Objective

Enable shared hook logic without sacrificing native inputs or outputs.

This phase proceeds per aligned event. Start with `PostToolUse` as soon as the
relevant native slices are stable; do not wait for unrelated Phase 3 event families.

#### Work package 4A: aligned input wrappers

For each justified common lifecycle event:

- add a native-arm input enum;
- retain complete native values;
- expose exact identity and raw payload;
- add only truly common optional accessors;
- represent multiple workspace roots;
- avoid fabricated tool/path data; and
- document capability differences from the catalog.

Remove or move recursive path heuristics that currently live in the common layer.

#### Work package 4B: aligned output wrappers

Add native-arm output enums for aligned events. The dynamic runtime must check that output and input arms match. A handler must be able to use a native-only response without abandoning the shared wrapper/runtime.

Do not lower through a generic output envelope.

#### Work package 4C: explicit-harness dynamic runtime

Implement:

- selected-harness dynamic execution;
- aligned-event execution with a selected harness;
- exact native parsing and emission underneath;
- optional event hints only where the event is dynamic;
- useful diagnostics/dump-parsed tooling; and
- feature-gated harness dependencies if that materially improves consumer builds.

CLI helpers may parse `--harness`, but the library API should accept a typed identifier rather than owning every application's CLI.

#### Work package 4D: semantic-intention audit

For every existing common output helper:

- identify which native effects it claims to represent;
- test whether lowering is total and lossless;
- distinguish no-op/approval/ask/deny/rewrite/stop;
- retain it, move it to an experimental module, move it downstream, or remove it; and
- record the rationale.

Do not introduce a stable replacement solely to finish this work package. The runner's local semantic result in Phase 5 is the preferred evidence-gathering mechanism.

#### Work package 4E: early runner API spike

As soon as the `PostToolUse` input/output wrappers and aligned runtime exist, port
one thin vertical path from the runner—preferably clean/no-op plus one diagnostic
outcome—using real native fixtures. Use the spike to revise awkward common/runtime
API before bulk native work makes it expensive. Keep the runner's domain result and
full migration in Phase 5.

#### Phase 4 exit gate

- Shared post-tool-use business logic can consume lossless native inputs.
- It can return the full native output for the selected harness.
- Mismatched native arms fail before emission.
- Unsupported effects are explicit.
- No conversion silently drops audience, rewrite, stop, or permission semantics.
- Production dynamic paths require explicit harness selection.
- The early runner spike exercises public `PostToolUse` APIs against real
  per-harness payloads.

### Phase 5 — Post-tool-use runner as the north-star consumer

#### Objective

Prove the stabilized design against the hook that motivated the library, without moving its domain policy into the library.

#### Guardrails during Phases 0–4

After every core/native/common/runtime change:

- the runner compiles;
- a small hermetic smoke suite runs;
- migration friction is recorded;
- temporary adapters have owners/removal phases; and
- core APIs are not expanded solely to eliminate reasonable native matches in runner lowering.

No new runner feature work should be mixed into protocol remediation unless it is needed to preserve behavior or expose a library design defect.

#### Work package 5A: local domain outcome

Define a runner-owned semantic result covering at least:

- clean/no action;
- auto-fixed;
- manual action required;
- formatter/linter/checker operational failure; and
- unsupported harness/event capability.

Keep tool policy, result classification, Pkl configuration, artifact strategy, and recursive path discovery local to the runner.

#### Work package 5B: per-harness lowering

Lower the local result explicitly into native output arms. For each harness, specify:

- what remains silent on clean success;
- what the user sees after an auto-fix;
- what context/instruction the agent receives;
- how manual action is requested or reported;
- how operational failure differs from a protocol denial;
- when diagnostics spill to an artifact; and
- which outcomes are unsupported.

Harness-specific matches in this final lowering are expected. They are preferable to pretending capabilities are uniform.

Antigravity `PostToolUse` may not expose enough information to drive this runner correctly. The outcome must then be explicitly unsupported, use a different documented event/state strategy, or require a separately justified session-state design. The common library must not invent missing fields.

#### Work package 5C: test redesign

Split runner testing into:

1. pure domain-classification unit tests;
2. orchestration tests using controlled fake executables;
3. exact runtime tests using one native fixture set per harness;
4. version-pinned real-tool compatibility tests; and
5. optional live-harness smoke tests.

Normalize locale, color, timestamps, and temporary paths. Force stable no-color output. Do not run arbitrary `PATH` versions against required checked-in goldens.

#### Work package 5D: performance and maintainability

Measure:

- hook startup;
- parsing and dispatch;
- clean no-op path;
- tool invocation overhead; and
- large invocation payloads.

Set budgets based on measured baselines rather than guesses. Avoid repeated parsing and unnecessary cloning. Document the addition of one new tool and one new harness lowering path to ensure the design remains maintainable.

#### Phase 5 exit gate

- Clean, auto-fixed, manual-action, and operational-failure paths work end to end.
- Every claimed harness receives its real native input/output behavior.
- User and agent messaging remain separate where supported.
- Diagnostics artifacts work.
- Required runner tests are hermetic.
- Real-tool tests are pinned or opt-in.
- Core/native/common/runtime have no Pkl or runner dependency.
- The public API feels clean even if lowering contains a harness match.
- Any candidate common semantic intention has an evidence-backed ADR; no promotion is required.

### Phase 6 — Packaging, documentation, and release stabilization

#### Objective

Make the remediated library consumable outside the workspace and ensure every support claim is backed by evidence.

#### Work package 6A: packaging

Apply the Phase 0 crate ownership/package-boundary decision, then decide whether
to add an optional umbrella/facade package and finalize publication mechanics. If
publishing existing crates:

- add `version + path` for internal dependencies;
- complete repository, license, description, keywords, and documentation metadata;
- define publish order;
- define Rust/MSRV policy;
- run `cargo package` for each publishable crate;
- add changelogs and release automation; and
- keep runner publication/versioning separate from core library release.

#### Work package 6B: generated support documentation

Publish/generated documentation should include:

- harness/event support matrix;
- handler-kind support matrix;
- audit snapshot and verification dates;
- typed, selected-harness, and aligned-event examples;
- plain-text output example;
- event-hint and detector semantics;
- native output escape-hatch examples;
- unchecked/raw output warning;
- contract contribution/refresh guide; and
- live conformance procedure.

All examples compile and run against contract fixtures in CI.

#### Work package 6C: release gates

The release workflow requires:

- `cargo fmt --check`;
- strict Clippy;
- hermetic workspace tests;
- catalog/meta-schema validation;
- Rust/catalog parity;
- exact wire tests;
- package verification;
- documentation examples;
- north-star runner suite; and
- the minimum live-conformance matrix below.

The first post-remediation release may include `native-source-reviewed` or
`command-runtime-beta` entries without live verification, but the generated matrix
and prose must label them as such. An event/binding may be labeled
`command-runtime-stable` only when:

- at least one valid invocation has been observed from the pinned harness
  generation;
- every materially distinct control outcome it claims (for example no-op,
  deny/block, rewrite, added context, stop, or value return) has a safe live
  observation or a reviewed infeasibility record;
- every special non-JSON transport it claims has been live observed; and
- the observation overlay records harness version, platform, date, and sanitized
  evidence.

Each harness described as stable must also have at least one complete end-to-end
command-hook smoke test. A harness that cannot meet this remains beta/source-
reviewed; inability to run it does not silently become “live verified.”

#### Phase 6 exit gate

- A fresh consumer can add the dependency and build a correct typed hook without reading repository internals.
- Every release claim comes from the generated matrix.
- Stable versus beta/source-reviewed labels satisfy the live-conformance policy.
- Publishable crates package successfully.
- The runner works against the same public/releasable APIs.
- No known P0/P1 protocol correctness issue remains.

### Phase 7 — Deferred runner extraction and ongoing maintenance

#### Objective

Reduce repository scope only after the library has proved itself, and make contract drift routine rather than exceptional.

#### Runner extraction criteria

Consider moving Pkl configuration and the runner downstream only when:

- core/common APIs have survived the complete migration;
- the runner consumes released public APIs;
- no runner-specific domain type remains in core/common;
- fixtures/tool catalog can move without weakening protocol coverage;
- a smaller library-side acceptance example preserves the cross-harness post-tool-use use case; and
- a pinned downstream compatibility job can continue to exercise releases.

Extraction is a separate decision and project, not an automatic final step.

#### Contract maintenance loop

For every upstream release or scheduled audit:

1. inspect event inventory and documentation/source changes;
2. create a candidate immutable snapshot;
3. refresh provenance and fixtures;
4. classify schema/process differences as additive, behavioral, breaking, or documentation-only;
5. record documentation/runtime conflicts;
6. update native types and outputs;
7. run catalog, conformance, runtime, runner, and live suites;
8. update the generated support matrix; and
9. release affected crates.

Upstream drift discovery may open a review task but must not rewrite snapshots or make ordinary offline CI nondeterministic.

## 11. Verification strategy

### 11.1 Required pull-request lanes

Every protocol change runs:

- metadata/schema validation;
- positive and negative contract fixtures;
- native parser/serializer conformance;
- exact process emission cases;
- core/native/common/runtime tests;
- compile-fail/type-safety tests where appropriate;
- examples; and
- hermetic runner smoke coverage.

### 11.2 Scheduled or opt-in lanes

Run separately:

- pinned formatter/linter catalog compatibility;
- live harness captures in temporary workspaces;
- upstream documentation/source drift discovery;
- packaging across supported Rust versions/platforms; and
- performance regression benchmarks.

### 11.3 Live conformance safety

Live harness tests must:

- use a temporary, non-sensitive workspace;
- document harness/version/platform;
- avoid network or destructive tool actions unless explicitly isolated;
- sanitize fixtures without changing structure;
- record input and exact output/process observations;
- separate command from HTTP/other binding behavior; and
- update the mutable observation/assurance overlay without mutating a frozen
  snapshot.

### 11.4 Coverage philosophy

Test both acceptance and rejection. Important negative cases include:

- wrong event discriminator;
- correct shape but wrong event hint;
- ambiguous shape without hint;
- output type for the wrong event;
- JSON where text is required;
- newline where zero bytes are required;
- missing required fields;
- invalid mutually exclusive output fields;
- no-op accidentally serialized as approval;
- protocol denial accidentally returned as process failure; and
- native capability lost through a common wrapper.

## 12. Migration and compatibility policy

The workspace is at `0.1.0`. Correctness takes priority over preserving protocol-invalid public APIs.

- Prefer replacing old types in coherent event-family changes.
- Keep temporary adapters private to the workspace where possible.
- If a public deprecation is necessary, document the exact semantic flaw and removal phase.
- Do not retain aliases that serialize an obsolete wire format.
- Do not accept casing aliases in canonical parsing unless a documented compatibility mode explicitly enables them.
- Keep `serde_json::Value`/unknown maps only where the contract is intentionally open, not as a substitute for modeling documented fields.
- Update examples in the same PR as the public API they demonstrate.
- Update the runner adapter in the same PR so main remains buildable.
- Catalog changes precede or accompany code; code must never silently outrun the catalog.

### 12.1 Expected API migration map

| Current surface | Target disposition |
| --- | --- |
| `hookkit_core::Harness` | Replace/refine with open `HarnessId` plus built-in convenience selection |
| `HookEventKey` | Replace with exact harness-scoped `EventId` and separate `AlignedEventKind` |
| `RawPayload` | Evolve into `RawInvocation` with retained bytes/JSON and exact context |
| Native `OutputEnvelope` types | Remove in favor of event/binding-specific outputs |
| `ClaudeEventOutput` and analogous permissive enums | Retain only as checked dynamic enums whose arm matches the input event |
| `CodexVersion` / `CodexFeatureSet` inferred from input | Remove unless a real upstream discriminator is cataloged |
| `CommonHookInput` and aligned input enums | Preserve the native-arm idea; rebuild from current native types |
| `CommonHookOutput`, `LoweringPolicy`, and universal intention lowering | Replace with native-arm output enums; move unproven intentions downstream/experimental |
| `run_native(harness, handler)` | Split into exact `run_event::<E>`, generic selected-harness, and explicit built-in dispatcher APIs |
| `run_common(harness, handler)` | Replace with event-specific `run_aligned_event::<K>(harness, handler)` |
| Current runtime `RuntimeContext` casing/path probes | Replace with the exact context contract in section 8.8 |
| Direct runtime logging to process streams | Replace with a configured diagnostics sink separated from protocol streams |

## 13. Risks and mitigations

### Schema inventory becomes a second stale implementation

Mitigation: immutable snapshots, pinned provenance, generated parity checks, scheduled drift review, and explicit audit dates.

### The catalog format becomes too ambitious

Mitigation: prove it on four difficult cases, keep v1 focused on JSON/text/empty/opaque plus process/HTTP metadata, and version the format.

### Schema reuse recreates a lowest-common denominator

Mitigation: event-root schemas per harness/event and reuse only for wire-identical components. Heritage metadata never implies semantic identity.

### Event-safe types cause type explosion

Mitigation: reuse private components and generic internal serializers, group public event modules consistently, and value correctness over one permissive envelope.

### Forward compatibility conflicts with schema strictness

Mitigation: explicitly separate normative catalog openness from Rust unknown-field preservation and test both.

### Detection consumes disproportionate effort

Mitigation: no automatic execution, modest catalog identification metadata, explicit harness runtime, and ambiguity as a valid result.

### Common output intent loses native behavior

Mitigation: native-arm output enums are mandatory; semantic intentions remain downstream until proven and must fail explicitly when unsupported.

### The runner continues to dominate changes

Mitigation: freeze unrelated runner features, maintain a small adapter/smoke suite, enforce dependency direction, and extract only after release stabilization.

### External tools make CI flaky

Mitigation: fake executables for required tests, pinned real-tool lanes, normalized output, and recorded versions.

### Mutable official docs make snapshots unverifiable

Mitigation: dated retrieval IDs, stable locators, pinned/captured revisions or
deterministic normalized hashes, retrieval recipes, and a mandatory low-confidence
unreproducible marker when none is possible.

## 14. Open decisions, with recommended defaults

These decisions should be settled by the named phase rather than assumed indefinitely.

| Decision | Recommended default | Deadline |
| --- | --- | --- |
| Exact YAML field spelling/meta-schema structure | Use the model in section 7 after the four-case spike | Phase 1A |
| Schema-to-Rust generation | Do not use initially | Reconsider after Phase 3 |
| HTTP and non-command runtime adapters | Out of the current implementation scope; retain catalog evidence | Reconsider only through a later ADR |
| Raw unchecked emission | Provide only if required, explicitly named/feature-gated | Phase 2 |
| Built-in harness enum versus open ID | Offer both convenience enum and open stable ID | Phase 2 |
| Stable common semantic intentions | None required initially; runner-local first | Phase 5 |
| Experimental intent module | Add only if it aids evaluation without implying stability | Phase 4 |
| Antigravity runner support | Explicitly unsupported unless sufficient documented data/path exists | Phase 5 |
| Package topology | Fix composable crate ownership in Phase 0; consider a facade separately | Phase 0/6 |
| Runner extraction | Defer until all extraction criteria pass | Phase 7 |

## 15. Suggested sessions and pull requests

Keep work reviewable. A likely sequence is:

1. Baseline artifact, CI lane classification, and ADR skeleton.
2. Contract meta-schema/validator spike with four difficult events.
3. Claude snapshot/index/input catalog.
4. Codex snapshot/index/input catalog.
5. Gemini CLI snapshot/index/input catalog.
6. Antigravity snapshot/index/input catalog.
7. Per-harness output/process catalog PRs, parallel where practical.
8. Core identity/raw-invocation/event-spec API.
9. Event-safe output and exact-process vertical slices.
10. Selected-harness resolution, hints, detector, and conformance runner.
11. Native post-tool-use slices for the target harnesses, with explicit
    unsupported status where a harness lacks required facts.
12. Lossless post-tool-use input/output wrappers and aligned runtime.
13. Early runner clean/diagnostic API spike.
14. Claude remaining event-family remediation PRs.
15. Codex remaining native rewrite PRs.
16. Gemini CLI remaining native rewrite PRs.
17. Antigravity remaining crate/event PRs.
18. Remaining justified common wrappers.
19. Existing semantic helper audit/removal/experimentalization.
20. Runner local domain outcome and full lowering migration.
21. Runner hermetic test split and performance work.
22. Packaging, generated docs, and release automation.
23. Post-release runner extraction decision.

Agents working in parallel should avoid editing central registry/report files by hand where possible. Generate indexes deterministically or assign one integration owner. A protocol implementation PR should normally cover one harness or one shared architectural primitive, not opportunistic changes across all harnesses.

## 16. Agent work-package handoff template

Every implementation handoff should state:

- phase and work-package ID;
- exact harnesses/events/bindings in scope;
- baseline commit;
- contract snapshot and sources consulted;
- contract files added or changed;
- fixture provenance;
- public APIs added, removed, or changed;
- supported input/output content kinds;
- exact process/HTTP outcomes implemented;
- positive, negative, compile-fail, and exact-wire tests added;
- support-report changes;
- runner compatibility/smoke result;
- commands executed;
- provisional assumptions or documentation conflicts;
- temporary adapters and their removal milestone; and
- work deliberately deferred.

A work package is not complete merely because Rust compiles. It must satisfy its phase gate and leave the catalog, implementation, tests, support report, examples, and consumer compatibility mutually consistent.

## 17. Overall definition of done

The realignment is complete when:

- the catalog inventories every current target harness/event input and every documented output/binding;
- non-JSON and process semantics are explicit;
- all catalog records have provenance and assurance;
- Claude, Codex, Gemini CLI, and Antigravity native crates conform for every
  implementation level promised by `stabilization-v1.yaml`, with other bindings
  explicitly catalog-only or unsupported;
- ordinary typed APIs cannot emit an output for the wrong event;
- dynamic/common execution always receives an explicit harness;
- event hints constrain ambiguous dynamic resolution and contradictions fail;
- best-effort detection exists independently of execution;
- common input and output wrappers retain full native values;
- no stable common semantic intention silently loses harness behavior;
- the post-tool-use runner is clean and correct through public APIs;
- required tests are offline and deterministic;
- live verification status is honest and visible, and every stable claim meets the
  minimum live-conformance policy;
- crates package successfully for a fresh consumer;
- documentation examples exercise typed and aligned workflows; and
- no known high-severity protocol correctness issue remains.

At that point, moving the runner out of the repository becomes safe to evaluate. It is not required to declare the library itself successful.
