# AGENTS.md

Primary agent instructions: see `./CLAUDE.md` in this same directory.

You MUST read the `CLAUDE.md` file in this same directory before making any changes.

## Build
cargo check

## Test
cargo test

## Lint
cargo clippy -- -D warnings

Once again, ALL architecture, conventions, and workflow rules are defined in `CLAUDE.md`.

## UI consistency review

Capture representative screenshots using synthetic fixtures for UI changes.
Have the owner approve screenshots before a separate adversarial UI consistency
review. Review against the criterion “like it was always there”: reuse existing
components, spacing, typography, colors, and interaction conventions. Approval
of an earlier design does not approve materially changed visuals.
