# ADR 0004: Outputs are event- and binding-specific

- Status: accepted
- Date: 2026-07-12
- Phase: 0

## Context

A universal JSON envelope permits invalid discriminators and conflates empty,
approval, denial, rewrite, process failure, text values, and HTTP responses.

## Decision

Every implemented event/binding has a finite native output type. Constructors
insert or validate protocol discriminators internally. No-op, explicit approval,
ask/defer, deny/block, rewrite, stop, audience channels, and operational failure
remain distinct. Unchecked raw output, if required, is conspicuously named and
outside checked support guarantees.

## Consequences

Legacy `OutputEnvelope` and protocol-invalid convenience builders are removed in
coherent migrations. Some public type proliferation is accepted in exchange for
wire correctness.
