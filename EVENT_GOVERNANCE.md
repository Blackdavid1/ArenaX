# Event Governance

This document describes the operator-facing policies that govern finalized
tournament records. Finalized match and tournament records are immutable except
through the explicit, audited correction flows described below.

## Finalized bracket corrections

Manual bracket repairs can change advancement and prize eligibility, so a
finalized bracket may only be corrected through a two-person review flow.

### Correction proposal

A correction proposal is created at the canonical tournament service boundary
(not in ad-hoc API handlers) and must record:

- **Before state** — the finalized bracket state being corrected.
- **After state** — the proposed replacement state.
- **Reason** — why the correction is needed.
- **Evidence reference** — a link or identifier to supporting evidence.
- **Proposer** — the operator who created the proposal.
- **Bracket version** — the bracket version the proposal was built against.

### Approval policy

- A proposal requires an **independent approver**: the approver must not be the
  proposer. Same-actor approval is rejected.
- The approver is recorded on the correction alongside the proposer so the
  change is fully auditable.
- A proposal is **rejected if the bracket version has changed** since the
  proposal was created (stale proposal). The proposer must re-propose against
  the current version.
- Finalized payout transitions cannot be silently rewritten; any change to a
  finalized payout must go through this same audited correction flow.

### Validation expectations

Automated coverage should exercise:

- Same-actor approval rejection.
- Stale proposal handling when the bracket version changes.
- Replay behavior (a proposal cannot be applied twice).
- Downstream standings reconciliation after an approved correction.
