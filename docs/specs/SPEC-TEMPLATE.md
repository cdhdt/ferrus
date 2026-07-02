# SPEC-XXXX: <module or feature name>

- **Status:** Draft | Accepted | Implemented | Superseded
- **Module:** `crate::<path>`
- **Linked ADRs:** ADR-XXXX, … (or "none")

## Role

One short paragraph: what this module is responsible for, and — just as
important — what it is explicitly NOT responsible for.

## Invariants

The properties that must always hold, safety-critical ones first. Each invariant
should be testable; note briefly how it is (or will be) tested. These are the
lines the implementation may not cross.

## Behavior

The observable contract: inputs, outputs, and the decision rules between them.
Describe the WHY and the rules, not a line-by-line paraphrase of the code (that
desynchronizes and ends up lying). Refusal and error cases are first-class
behavior — document them here, not as an afterthought.

## Known pitfalls

Traps, platform quirks, values that drift across versions, things that look
correct but are not. This is where hard-won knowledge lives.

## Out of scope

What this spec deliberately does not cover, and which later phase owns it.
