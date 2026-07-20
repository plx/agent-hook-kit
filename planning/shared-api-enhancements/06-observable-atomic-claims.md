# Shared API Enhancement 06: Optional Observable Atomic Claims

<!-- markdownlint-disable MD013 -->

- Status: deferred pending demand
- Priority: P2 optional follow-up
- Dependencies: items 01 through 05 should be complete
- Primary crate: `hookkit-session-state`
- Gate: reopen only when a concrete consumer requires inspectable claim metadata
- Controlling plan: [Shared API Enhancement Plan](../shared-api-enhancement-plan.md)

## Objective

If demonstrated consumer requirements justify it, add a generic state primitive that combines `ClaimSet`'s cheap atomic first-writer decision with opt-in, inspectable metadata.

This task is intentionally conditional. `codex-claude-rules` currently needs only an immediate first-writer-wins result and does not consume a loaded-rule projection. The existing hashed-key `ClaimSet` is therefore the correct default.

## Deferral decision (2026-07-20)

**Outcome: deferred pending demand.** Items 01 through 05 did not produce a
consumer that reads claim metadata, and no inspectable metadata schema is
currently required. `codex-claude-rules` still needs only the atomic claimed or
already-claimed result supplied by `ClaimSet`.

Decision-gate record:

1. No current consumer reads claim metadata.
2. No observable fields or stable schema have been identified.
3. Plaintext persistence safety cannot be evaluated without a concrete schema.
4. No journal replacement is needed because the existing marker-only claim is
   the appropriate and cheaper semantic fit.
5. Existing `claimed\n` markers require no compatibility change because no new
   representation is being introduced.

No implementation or persisted-state change was made. Reopen this item only
when a concrete consumer can answer the gate below; general interest in richer
state inspection is not sufficient by itself.

## Reopening gate

Before changing code, record answers to:

1. Which consumer reads claim metadata?
2. What fields must be observable?
3. Is the data safe to persist in plaintext inside the private session-state directory?
4. Why are `SetJournal<T>` or an existing typed entity too heavy or semantically wrong?
5. What compatibility behavior is required for existing marker-only claims?

If no concrete reader and schema exist, complete this task by documenting continued deferral. Do not add a speculative API.

## Required design if the gate passes

Prefer a generic extension such as an atomic claim with caller-supplied serializable metadata. Exact names are not frozen, but the API should support:

- `try_claim` retaining its current behavior;
- an opt-in `try_claim_with<T: Serialize>` or equivalent;
- exactly one successful claimant for a key under concurrency;
- hashed filenames so keys are not exposed as path components;
- an inspectable typed snapshot or iterator for metadata-bearing claims; and
- explicit handling of marker-only legacy claims and unreadable/corrupt metadata.

Do not add a `LoadedRules`-specific insertion primitive unless the demonstrated requirement cannot be expressed generically.

## Atomicity and crash semantics

The design must state what happens if a process crashes after exclusively creating a claim but before fully writing metadata.

The safe baseline is:

- the claim remains claimed once exclusive creation succeeds;
- a second process must never win because metadata is incomplete;
- metadata inspection may report a typed corrupt/incomplete record;
- inspection failure must not silently release or recreate the claim; and
- cleanup or repair, if supported, must be explicit and separately synchronized.

If the implementation can atomically publish complete metadata without weakening exclusive creation, document and test the mechanism. Do not depend on a rename operation that overwrites an existing claimant.

## Privacy and compatibility

- Metadata is opt-in and must not include the unhashed key unless the caller supplies it deliberately.
- Preserve private directory permissions and symlink protections.
- Existing `claimed\n` marker files must remain valid.
- Do not change the default on-disk representation for `try_claim` without a migration reason.
- A snapshot should distinguish marker-only, metadata-bearing, and corrupt claims.
- Do not promise ordering for independent claims.

## Required tests if implemented

Cover:

- many concurrent claimants producing exactly one winner;
- metadata from the winner being inspectable;
- marker-only claim compatibility;
- metadata not implicitly exposing the original key;
- serialization failure before claim publication;
- crash/incomplete metadata behavior using a controlled fixture;
- corrupt metadata surfaced without releasing the claim;
- private permissions and symlink defenses; and
- unchanged behavior of `ClaimSet::contains` and `try_claim`.

Run at minimum:

```bash
cargo test -p hookkit-session-state
cargo clippy -p hookkit-session-state --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Exit criteria

One of the following outcomes is required:

1. **Deferred:** no concrete consumer justifies the API, and the plan records that no implementation was made; or
2. **Implemented:** a generic, opt-in metadata claim API passes concurrency, crash, privacy, and compatibility tests without changing the cheap marker-only default.
