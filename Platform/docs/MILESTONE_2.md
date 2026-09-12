# Milestone 2 — Safe Intake and Snapshot Completion Record

Status: complete  
Implementation language: Rust  
Source-project mutation: none  
Imported-code execution: none  
Network required at runtime: none

## Delivered architecture

Milestone 2 adds the `qbm-intake` crate and extends the domain, application,
store, and CLI layers created in Milestone 1.

```text
registered source
  -> read-only type detection
  -> safety and resource policy
  -> deterministic inventory
  -> included bytes frozen in SHA-256 storage
  -> inventory stage waits for exact-hash approval
  -> artifact integrity verification
  -> immutable source snapshot
  -> source-snapshot stage waits for exact-hash approval
```

The existing Python Q-BenchMed core remains independent. No Python module is
imported by this Rust workspace.

## Supported intake sources

- local directory;
- ZIP archive;
- uncompressed TAR archive;
- gzip-compressed TAR (`.tar.gz` or `.tgz`) archive.

Archive content is read from the content-addressed frozen copy. It is not
extracted into the registered project, platform workspace, or another shared
directory.

## Deterministic inventory

Every inventory records:

- run and project identities;
- source type and optional source-container hash;
- canonical intake-policy hash;
- normalized relative entry paths;
- file/directory/symlink/special kind;
- included, excluded, or blocked disposition;
- stable reason codes;
- included file sizes and SHA-256 identities;
- included, excluded, blocked, and byte totals;
- a canonical inventory identity.

Entries are sorted before identity generation. Creation timestamps and random
record UUIDs do not affect the inventory hash. Repeated scans of unchanged
content with the same project and policy produce the same inventory hash.

## Default policy

The default policy excludes common generated or private metadata names at any
depth:

```text
.DS_Store .git .hg .qbenchmed .svn __pycache__ node_modules target
```

Users may add excluded names, excluded path prefixes, or included path prefixes.
The CLI also exposes bounded overrides for:

- observed entry count;
- total included bytes;
- one-file size;
- path depth;
- archive compression ratio.

## Safety properties

- Folder symlinks are recorded and never followed.
- Archive symlinks and special entries are not materialized.
- Absolute paths, parent traversal, Windows drive prefixes, NULs, and non-UTF-8
  paths fail closed.
- Duplicate archive paths are blocked.
- Oversized, over-depth, and excessive-expansion entries are blocked.
- An entry-count overflow fails the scan because a partial inventory would be
  misleading.
- Included data is streamed into content-addressed storage rather than loaded as
  a complete project in memory.
- Archive member reads are bounded and checked against declared size.
- Snapshot creation re-hashes every referenced artifact and fails on tampering.
- Any blocked entry prevents snapshot creation.

## Governance and persistence

Database schema version 2 adds:

```text
inventories
source_snapshots
run_snapshots
```

`qbm intake scan` persists the inventory manifest and records its hash as the
current `inventory` stage output. `qbm snapshot create` requires that exact hash
to be in the approved state. A changed scan invalidates the earlier approval
through the existing stage-output state machine.

The immutable snapshot binds normalized paths to frozen SHA-256 artifacts. Its
canonical identity is recorded as the `source-snapshot` stage output, so the
snapshot itself can be approved or rejected before later scanners consume it.

## CLI delivered

```text
qbm intake scan|show|list
qbm snapshot create|show|list
```

`intake scan` policy flags:

```text
--exclude-name
--exclude-prefix
--include-prefix
--max-entries
--max-total-bytes
--max-single-file-bytes
--max-depth
--max-compression-ratio
```

Project registration now accepts either a directory or a supported archive.

## Verification completed

```text
cargo fmt --all -- --check
  PASS

cargo clippy --workspace --all-targets --offline -- -D warnings
  PASS

cargo test --workspace --offline
  PASS — 21 tests
```

Covered behavior includes deterministic scans, default exclusions, symlink
refusal, path normalization, folder/TAR/ZIP intake, size blocking, repeatable
snapshot identity, exact approval gates, frozen-source behavior, durable reopen,
artifact integrity, stale approvals, event-chain integrity, and idempotency.

A real CLI smoke workflow also completed against the Platform workspace:

```text
initialize isolated store
  -> register source
  -> start governed run
  -> scan 20 included files and exclude target/
  -> approve exact inventory hash
  -> create immutable snapshot
  -> list persisted snapshot
  -> verify event chain
```

## Deliberately not included yet

Milestone 2 freezes and governs the evidence input. It does not yet interpret
YAML, JSON, Rust, neural models, APIs, domain rules, or outcomes. Raw project
graph construction and generic structural audit are Milestone 3. Adapters and
domain-specific semantics follow in later milestones.
