# Milestone 3 — Raw Graph and Generic Audit Completion Record

Status: complete  
Implementation language: Rust  
Imported-code execution: none  
Input prerequisite: exact approved source snapshot

## Delivered architecture

Milestone 3 adds `qbm-audit`, scanner-neutral graph contracts, database schema
version 3, application orchestration, and CLI commands.

```text
approved source-snapshot hash
  -> integrity-checked frozen artifacts
  -> non-executing format scanners
  -> deterministic raw project graph
  -> raw-graph exact-hash approval
  -> generic policy engine
  -> evidence-linked audit report
  -> generic-audit exact-hash approval
```

No scanner reads the mutable registered source folder. It consumes only the
content-addressed artifacts named by the approved snapshot.

## Supported scanners

### JSON

Parses the complete JSON value tree, records object/array/scalar structure,
identifiers, explicit references, and JSON Pointer provenance.

### YAML

Parses YAML safely as data, including multi-document streams. It records the
same scanner-neutral structure and preserves a deterministic document namespace.

### TOML

Parses TOML into the same structured-value graph without executing build tools,
scripts, or project code.

### Rust

Uses Rust syntax parsing only. It records named items, inline and external
modules, imports, syntax paths, and source line/column evidence. It does not call
`rustc`, Cargo build scripts, procedural macros, linkers, binaries, or tests.

Unsupported formats still receive file nodes and immutable provenance; they are
available to future adapters without a false claim that their semantics were
understood.

## Raw graph contract

The deterministic graph contains:

- synthetic project root;
- one file node per snapshot artifact;
- structured document and Rust syntax nodes;
- containment, explicit-reference, import, and module-declaration edges;
- resolved target IDs where a unique target exists;
- unresolved edges retaining the original reference text;
- SHA-256 artifact, normalized path, syntax pointer, and optional line/column
  evidence;
- non-fatal parser diagnostics;
- exact graph-policy and source-snapshot identities.

Node, edge, graph, finding, and report IDs are hashes of canonical semantic
fields. Timestamps are excluded, so rebuilding unchanged evidence with the same
policy produces the same identities.

## Default graph policy

Resource boundaries:

```text
maximum parsed supported file: 16 MiB
maximum graph nodes:            1,000,000
maximum graph edges:            2,000,000
```

Default identifier keys:

```text
$id id
```

Default explicit reference keys:

```text
$ref depends_on extends ref reference
```

Default generic definition containers:

```text
$defs components definitions nodes rules workflows
```

CLI flags can extend these key sets and override the resource bounds. Domain
adapters will later supply narrower, versioned policies rather than making the
generic scanner guess domain meaning.

## Generic findings

Every finding has a canonical ID, code, severity, title, detailed message,
remediation, affected graph nodes, and immutable evidence locations.

Delivered policies:

- scanner parse/UTF-8/resource diagnostics;
- duplicate identifiers with Rust file scoping;
- byte-identical files at multiple paths;
- unresolved or ambiguous explicit references;
- unresolved Rust external module declarations;
- external or unresolved Rust imports as informational findings;
- iterative strongly connected component detection for reference cycles;
- unreferenced generic document definitions;
- critical provenance coverage enforcement.

Reachability is intentionally conservative. The generic layer reports document
definitions lacking incoming explicit references as informational, not as proof
that they are dead. Domain adapters can define authoritative roots later.

## Governance and persistence

Database schema version 3 adds:

```text
raw_graphs
run_graphs
audit_reports
run_reports
```

`qbm graph build` requires the exact `source-snapshot` stage output to be
approved and attached to the same run. It records the graph ID as `raw-graph`.

`qbm audit run` requires the exact `raw-graph` stage output to be approved and
attached to the same run. It records the report ID as `generic-audit`.

A changed snapshot or graph therefore returns the dependent stage to
`waiting_approval`; approval of an earlier hash cannot silently authorize new
evidence.

## CLI delivered

```text
qbm graph build|show|list
qbm audit run|show|list
```

Graph policy flags:

```text
--identifier-key
--reference-key
--definition-container
--max-parse-file-bytes
--max-nodes
--max-edges
```

All output is structured JSON and can be filtered or transformed by downstream
automation without scraping prose.

## Verification completed

```text
cargo fmt --all -- --check
  PASS

cargo clippy --workspace --all-targets --offline -- -D warnings
  PASS

cargo test --workspace --offline
  PASS — 29 tests
```

The tests cover all four scanners, deterministic graph/report identities,
non-mutating operation, parser diagnostics, duplicate/unresolved/cycle/
reachability policies, iterative cycle analysis, exact predecessor approvals,
durable reopen, and event-chain verification.

A real CLI continuation of the Milestone 2 smoke run completed:

```text
approve immutable snapshot
  -> build graph: 488 nodes, 543 edges, 0 parser diagnostics
  -> approve exact graph hash
  -> run generic audit: 56 informational findings, 0 warning/error/critical
  -> verify complete event chain
```

The 56 informational observations are mostly conservative Rust import or
definition reachability notes. They are not represented as proven defects.

## Deliberately not included yet

Milestone 3 is domain-blind by design. It does not yet know that a particular
YAML object is a PATHEX rule, ONCOVA guard, biomarker definition, neural model,
or benchmark objective. The declarative biomedical Adapter SDK and semantic
projection contracts are Milestone 4.
