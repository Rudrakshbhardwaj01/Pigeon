# Contributing to Pigeon

Thank you for considering a contribution. This guide explains how the project
is built, tested and reviewed so your change lands quickly.

## What Pigeon is

An offline-first, privacy-first optical file transfer platform: files become
animated QR-code GIFs and are reconstructed **byte-for-byte** on another
device. No server, no account, no cloud. Read the
[README](README.md) and [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
before your first change — they describe the boundaries the project
maintains on purpose.

## Ground rules

1. **The Rust core owns correctness.** Protocol logic, validation,
   cryptography and reconstruction live in `crates/pigeon-core`. The WASM
   crate is a thin translation layer; TypeScript holds only UI state and
   worker plumbing. Do not reimplement engine logic in JavaScript.
2. **No backend, no telemetry.** Pigeon must keep working fully offline with
   no remote requests (enforced by CSP and verified by an E2E test). Do not
   add network calls, analytics or remote fonts/scripts.
3. **Wire compatibility is deliberate.** The protocol has a version byte and
   reserved flag space (`docs/PROTOCOL.md`). Changes that alter the wire
   format need a documented decision record, explicit rejection rules for
   unknown inputs, and a compatibility note.
4. **Cryptography is not improvised.** Use maintained crates (RustCrypto),
   keep authentication before any plaintext use, and add adversarial tests
   for every new parser or format.
5. **Honesty over polish.** Progress reflects engine state; failures are
   explicit; estimates state their assumptions. Never present an
   unverified file as success and never fabricate numbers.

## Getting started

Prerequisites: Node.js ≥ 20 (npm only), and Rust with `wasm32-unknown-unknown`
+ wasm-pack if you change the engine (the compiled WASM is committed, so
frontend-only work needs no Rust).

```sh
git clone https://github.com/Rudrakshbhardwaj01/Pigeon
cd Pigeon
npm install
npm run dev          # http://127.0.0.1:5173
```

After touching the Rust crates, regenerate the committed engine:

```sh
npm run build:wasm
```

## Commands

| Command | What it does |
|---|---|
| `npm run dev` | Vite dev server |
| `npm run build` | Type check + production build (`apps/web/dist`) |
| `npm run preview` | Serve the production build on `127.0.0.1:4173` |
| `npm run test:rust` | Rust unit + native optical roundtrip suites |
| `npm run test:web` | Vitest unit/component tests |
| `npm run test:e2e` | Playwright real-browser suite (incl. offline roundtrip) |
| `npm run typecheck` | `tsc -b` |
| `npm run lint` | ESLint |
| `cargo fmt --all` | Format Rust code (CI enforces with `--check`) |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint Rust (CI enforces) |

Benchmarks (release mode) and the large multi-block FEC test:

```sh
cargo test -p pigeon-core --release --test bench -- --ignored --nocapture
```

## Where things live

```text
crates/pigeon-core/      Engine: protocol, FEC, QR, GIF, crypto, recovery
crates/pigeon-wasm/      wasm-bindgen API surface (translation only)
apps/web/src/            React app, workers, service worker, WASM bridge
apps/web/tests/e2e/      Playwright suite
docs/                    Protocol, architecture, security, threat model
tests/fixtures/          Shared E2E fixtures
```

Architecture changes start with a decision record in
`docs/DECISIONS.md` (context → problem → options → decision → consequences →
rejected alternatives).

## Testing expectations

- Every bug fix lands with a regression test.
- New engine behaviour needs unit tests plus, where it touches the wire, an
  entry in the native roundtrip suite (`crates/pigeon-core/tests/roundtrip.rs`).
- New UI behaviour needs a component test and, for complete workflows, an E2E
  test. Screenshots alone are not verification.
- Security-sensitive parsing (new formats, new envelope fields) needs
  adversarial cases: truncation, corruption, oversized values, unknown ids,
  and cross-transfer mixing.
- Run the full suite before opening a PR; CI runs the same commands.

## Pull requests

1. Open or comment on an issue first for anything beyond a small fix, so
   scope is agreed before code is written.
2. Keep PRs focused: one logical change, described against its motivation.
3. Include: what changed, why, how it is tested, and any documentation or
   changelog updates (`CHANGELOG.md` entry under *Unreleased*).
4. Ensure `cargo fmt`/`clippy`, `npm run typecheck`, `npm run lint` and the
   test suites pass locally. CI replicates them.
5. Update documentation in the same PR when behaviour or commands change.
   Stale documentation is treated as a bug.

## Protocol changes

The wire protocol is the project's longest-lived contract. Proposals must
state: compatibility with v1 receivers, the rejection behaviour for packets
that use the new feature incorrectly, and the resource limits introduced.
Additive features use the reserved flag space; wire-incompatible changes bump
the version byte. See `docs/PROTOCOL.md` §7.

## Issue reporting

Use the issue templates. For security vulnerabilities, follow
[`SECURITY.md`](SECURITY.md) rather than opening a public issue.

## License

By contributing you agree that your contributions are licensed under the
project's [MIT License](LICENSE).
