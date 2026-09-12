# Milestone 1 — Foundation Completion Record

Status: complete  
Implementation language: Rust  
Source-project mutation: none  
Network required at runtime: none

## Delivered workspace

```text
Platform/
├── apps/qbm-cli
├── crates/qbm-app
├── crates/qbm-canonical
├── crates/qbm-domain
├── crates/qbm-store
├── Cargo.toml
├── Cargo.lock
├── LICENSE
├── README.md
└── ROADMAP.md
```

## Delivered invariants

### Portable domain identities

Project, stage, and actor IDs permit only a bounded portable character set.
Run, event, and approval IDs use UUIDs. SHA-256 values are validated and
normalized before use.

### Canonical identities

Structured identities are serialized with recursively ordered object keys and
then hashed with SHA-256. Streaming file hashing avoids loading large artifacts
into memory.

### Content-addressed artifacts

Artifacts are stored at:

```text
artifacts/sha256/<first-two>/<next-two>/<full-sha256>
```

Identical content is deduplicated. Metadata is recorded in SQLite. Temporary
files and destination files live below the same platform data root so commit is
an atomic rename on the normal local filesystem path.

### Durable projects and runs

Registered project paths are canonicalized and recorded. Registration requires
an existing directory and does not write to it. Runs record project, mode,
state, creation time, and last state change.

### Tamper-evident event chain

Every run begins with `run_created`. Each later event contains a monotonic
sequence and the preceding event hash. `qbm run verify-events` recomputes the
canonical chain and rejects missing, reordered, or changed events.

### Hash-bound approvals

A completed material stage records one current output hash and enters
`waiting_approval`. Approval, rejection, and change requests must name that
exact hash. When a stage produces new output, approval against the previous hash
fails as stale.

Rejection and request-changes decisions require a reason. Decisions and state
changes are committed atomically with their event records.

## SQLite schema

```text
projects
runs
run_events
stage_outputs
approvals
artifacts
```

Foreign keys, write-ahead logging, full synchronous durability, a busy timeout,
and schema version 1 are enabled when the store opens.

## CLI delivered

```text
qbm init

qbm project add|list|show

qbm run start|list|show
qbm run approve|reject|request-changes
qbm run approvals|events|verify-events

qbm stage complete

qbm artifact put|show
```

All commands accept an explicit global `--data-dir`. `QBM_DATA_DIR` is the
secondary choice; `.qbenchmed` is the local fallback.

## Verification completed

```text
cargo fmt --all -- --check
  PASS

cargo clippy --workspace --all-targets --offline -- -D warnings
  PASS

cargo test --workspace --offline
  PASS — 12 tests
```

Tested behaviors:

- portable identifier validation;
- UUID round trip;
- digest normalization;
- canonical key-order independence;
- streaming/direct hash parity;
- artifact deduplication;
- stale approval rejection;
- duplicate-decision rejection;
- idempotent identical stage-output retry;
- durable approved stage state;
- event-chain verification;
- required rejection reason;
- application-level project/run/approval lifecycle.

A real CLI smoke test also completed:

```text
initialize store
  -> register this Platform directory
  -> start governed run
  -> record inventory output
  -> approve exact output hash
  -> verify event chain
  -> store README as a content-addressed artifact
```

## Deliberately not included yet

Milestone 1 does not claim to provide:

- folder/archive quarantine and inventory;
- immutable source snapshots;
- source scanners or raw project graph;
- adapters or audit policies;
- browser UI or HTTP API;
- behavioral execution;
- project optimization;
- QUBO/quantum execution.

These are sequenced in `ROADMAP.md`; their absence is not hidden behind the
foundation CLI.

## Next milestone

Milestone 2 will add:

1. safe local folder intake;
2. archive quarantine with traversal and expansion controls;
3. deterministic inventory records;
4. include/exclude policy;
5. symlink handling;
6. streaming hashes and unchanged-file reuse;
7. immutable source snapshot identity;
8. CLI commands to import, inspect, and approve an inventory/snapshot.
