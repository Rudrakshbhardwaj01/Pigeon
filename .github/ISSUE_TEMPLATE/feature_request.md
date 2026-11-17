---
name: Feature or improvement proposal
about: Suggest a capability or a better way to do something
title: "[proposal] "
labels: enhancement
assignees: ""
---

**Problem to solve**

What can a user not do today, or what is needlessly hard? Concrete scenarios
beat abstract feature lists.

**Proposed approach**

What should change, at what layer? Note which boundary it touches:

- [ ] Rust core / wire protocol (needs a compatibility statement — see
      `docs/PROTOCOL.md` §7 and `CONTRIBUTING.md` → Protocol changes)
- [ ] WASM bridge
- [ ] Web UI
- [ ] Docs / DX / CI

**Constraints the project maintains**

Pigeon is offline-first and local-only: no backend, no accounts, no
telemetry, no cloud. Protocol logic stays in Rust. Proposals that preserve
these constraints are much easier to accept.

**Alternatives considered**

Including workarounds you already use helps calibrate the real need.

**Willingness to contribute**

Would you like to implement this yourself? The contribution guide explains
the workflow.
