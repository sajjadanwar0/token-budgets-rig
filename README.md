# token-budgets-rig

A small **N=1 deployment case study** for the [`token-budgets`](https://github.com/sajjadanwar0/token-budgets)
crate: it wires the affine `Budget` / `Reservation` types into two real Rust
agent frameworks — [Rig](https://github.com/0xPlaygrounds/rig) (async-task) and
**AutoAgents** (actor-model) — and shows that a single session token budget is
held across a multi-agent workflow, with the budget split among sub-agents in a
way the compiler refuses to let them bypass.

This crate is a demonstration artifact, not a benchmark suite. It backs the
deployment section of the token-budgets paper and is exercised by that
project's `reproduce.sh` (Phase 5b offline, Phase 8 live).

## What it demonstrates

Two independent guarantees, each checkable on its own:

1. **Runtime cap-enforcement (offline, no API key).** A parent agent holds a
   session budget and hands each sub-agent a `Reservation` slice. Across a
   single agent and across eight concurrent sub-agents, the total spend never
   exceeds the session cap — the cap holds even under concurrent fan-out.

2. **Compile-time non-bypassability (offline, no API key).** A `Reservation`
   is an affine resource: a sub-agent cannot clone its slice or reuse it after
   it has been moved. These are enforced by the type system, so the violations
   are *compile errors*, verified as `trybuild` snapshots:
   - cloning a `Reservation` → `error[E0599]` (no `clone` method)
   - reusing a moved `Reservation` → `error[E0382]` (use after move)

The live examples (Phase 8) additionally show the same cap holding against the
real Anthropic API through both frameworks.

## Layout

```
token-budgets-rig/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml          # pins the rustc used for trybuild snapshots
├── src/                         # Reservation wiring for Rig + AutoAgents
├── examples/                    # live demos (need an API key)
│   ├── delegation_demo.rs       # Rig: parent delegates to a sub-agent
│   ├── fanout_demo.rs           # Rig: concurrent sub-agent fan-out
│   ├── workload_multiturn.rs    # Rig: multi-turn session under one cap
│   └── autoagents_fanout.rs     # AutoAgents: actor-model fan-out under one cap
└── tests/
    ├── cap_enforced.rs          # offline: single-agent cap held
    ├── fanout_cap.rs            # offline: 8 concurrent sub-agents, cap held
    ├── affine_reservation.rs    # trybuild harness for the compile-fail cases
    └── compile_fail/
        ├── reservation_no_clone.rs(.stderr)   # E0599
        └── reservation_no_reuse.rs(.stderr)   # E0382
```

## Prerequisites

- **Rust** — the toolchain is pinned in `rust-toolchain.toml` (currently
  `1.95.0`); `rustup` installs it automatically on first build. The pin exists
  so the `trybuild` compile-error snapshots match a known compiler (see
  *trybuild snapshots* below).
- **For `autoagents_fanout` only:** system OpenSSL + `pkg-config`
  (Debian/Ubuntu: `sudo apt install pkg-config libssl-dev`), and the
  `anthropic` feature.
- **For any live example:** an Anthropic API key:
  ```bash
  export ANTHROPIC_API_KEY=sk-ant-...
  ```

## Quick start — offline (no API key)

These reproduce both guarantees and cost nothing:

```bash
# 1. Runtime cap-enforcement: single agent + 8 concurrent sub-agents
cargo test --release --test cap_enforced --test fanout_cap

# 2. Compile-time non-bypassability: a sub-agent cannot clone/reuse its slice
cargo test --release --test affine_reservation
```

Both should report `test result: ok`. The second compiles the
`tests/compile_fail/*.rs` cases and checks that each fails with the expected
compiler error.

## Live examples (need `ANTHROPIC_API_KEY`)

Each prints a `CAP RESPECTED` line when the session budget held. These make
real API calls (a few cents total).

Rig framework:
```bash
cargo run --release --example delegation_demo
cargo run --release --example fanout_demo
cargo run --release --example workload_multiturn
```

AutoAgents framework (actor-model; needs OpenSSL + the `anthropic` feature):
```bash
cargo run --release --example autoagents_fanout
```
On success it prints `CAP RESPECTED ACROSS ALL AGENTS`, showing one shared cap
held across concurrent actor-model agents.

## trybuild snapshots

The compile-fail tests compare actual compiler output against the committed
`tests/compile_fail/*.stderr` files. Compiler diagnostics are **rustc-version
specific**, so these snapshots are tied to the toolchain pinned in
`rust-toolchain.toml`. If you bump that toolchain (or `cargo test
--test affine_reservation` reports `.stderr` drift), regenerate them with the
same flags the tests run under and commit the result:

```bash
TRYBUILD=overwrite cargo test --release --test affine_reservation
cargo test --release --test affine_reservation        # confirm: ok
git add rust-toolchain.toml tests/compile_fail/*.stderr
git commit -m "trybuild: refresh snapshots for pinned toolchain"
```

Keep `rust-toolchain.toml` and the `.stderr` files in sync and committed
together — that is what makes the compile-fail guarantee reproducible on
another machine.

## How it fits the reproduction

The token-budgets `reproduce.sh` drives this crate automatically:

- **Phase 5b** runs the offline tests above (`cap_enforced`, `fanout_cap`,
  `affine_reservation`) — always, with no API key.
- **Phase 8** (`./reproduce.sh --with-live`) runs the live Rig and AutoAgents
  examples, checking for the `CAP RESPECTED` lines.

To run this crate from a checkout other than the one `reproduce.sh` clones,
point it at your copy:
```bash
export RIG_DIR=/path/to/token-budgets-rig
```

## License / status

Early case-study crate (version `0.0.0`); the API is illustrative and may
change. See the parent [`token-budgets`](https://github.com/sajjadanwar0/token-budgets)
crate for the core affine budget types this builds on.
