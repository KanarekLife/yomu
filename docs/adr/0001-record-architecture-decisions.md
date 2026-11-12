# 0001. Record architecture decisions

- Status: accepted
- Date: 2026-11-12

## Context

The project spans five platforms, a Rust core, a web UI and several
Japanese-language subsystems. Decisions need to stay discoverable and their
reasoning must survive after the people who made them have moved on.

## Decision

We use Architecture Decision Records (Michael Nygard's format) in
`docs/adr/`, numbered sequentially, one decision per file, using
`0000-template.md`. ADRs are immutable once accepted; a changed decision gets a
new ADR that supersedes the old one. The index lives in `docs/adr/README.md`.

Work is organised around ADRs: a feature that needs an architectural choice
starts with an ADR (status *proposed*), lands with the ADR *accepted*, and the
ADR's "Follow-ups" section lists the work that is deliberately left for later.

## Consequences

- Reviewers can check code against an explicit, written intent.
- Some overhead for each significant change.
