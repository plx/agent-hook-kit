# ADR 015: Runner extraction remains deferred

- Status: accepted
- Date: 2026-07-12
- Phase: 7

The Pkl configuration and post-tool-use runner remain in this repository for the
contract-first reboot. They consume public workspace APIs and no runner domain
type has moved into core/common, but extraction criteria are not all satisfied:

- the legacy common semantic facade still supports part of the migration;
- the runner has not yet consumed a published release of the crates;
- downstream pinned compatibility automation cannot exist before that release;
- moving the large tool fixture/catalog corpus now would weaken the acceptance
  consumer during review.

Reconsider extraction only after a released API has survived a downstream runner
migration. A library-side aligned PostToolUse example and pinned downstream job
must be in place before any files move. Extraction will be its own project and
will not rewrite this repository's frozen protocol evidence.
