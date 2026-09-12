# Declarative Adapter Authoring Guide

This guide explains how to connect a new biomedical project family to
Q-BenchMed Platform without adding executable plugin code. Q-BenchMed is **A
Modular Open-Source Quantum–Classical Benchmarking Framework for Biomedical
Input Optimization**. An adapter is a strict JSON description that tells the
trusted Rust interpreter how to recognize raw graph evidence, map it into
reviewed biomedical terms, test policies, and publish useful views.

Start from
[`examples/adapters/generic-manifest-v1.json`](../examples/adapters/generic-manifest-v1.json).
Copy it outside the installed registry, give it a new adapter ID, and change one
section at a time while repeatedly running `adapter package check`.

## What goes into an adapter

Do not copy an entire project, database, model, PDF collection, YAML tree, or
source repository into the adapter. The platform freezes and scans each project
once. The adapter contains only reusable interpretation rules and small
synthetic fixtures.

```text
project folder/archive
  -> immutable source snapshot
  -> scanner-neutral raw graph
  -> your declarative adapter
  -> namespaced semantic graph + findings + projections
```

Use one adapter for one stable biomedical semantic vocabulary. PATHEX, ONCOVA,
and other biomedical project families should normally have different adapter
IDs. Multiple adapters can be selected in one plan; every emitted item remains
attributed to its exact package hash.

## Step 1 — Inspect real raw evidence

First run the ordinary intake and graph workflow on a representative project,
then inspect the JSON from:

```bash
qbm graph show <raw-graph-hash>
```

For candidate raw nodes, record only fields the generic scanner actually emits:

- `format`, such as `json`, `yaml`, `toml`, `rust`, or `python`;
- `kind`, such as `object`, `array`, `scalar`, `file`, `rust_item`, or
  `rust_module`;
- normalized `relative_path` and syntax `pointer` evidence;
- `label`, optional `identifier`, and `is_definition`;
- scanner-neutral `properties`; and
- resolved raw edges such as `contains`, `references`, `imports`, or
  `declares_module`.

Never design a selector around a field that appears only in the original file
but is absent from the raw graph. If a domain needs additional safe syntax
extraction, that belongs in a reviewed Rust scanner change, not in an adapter.

## Step 2 — Define identity and capabilities

Use the current immutable versions:

```json
{
  "schema_version": "qbm.adapter-package/v1",
  "manifest": {
    "schema_version": "qbm.adapter-manifest/v1",
    "adapter_api_version": "qbm.adapter-api/v1",
    "id": "your-adapter",
    "version": "1.0.0",
    "minimum_platform_schema": "qbm.domain/v1"
  }
}
```

IDs are portable lowercase names. Semantic types, relationships, and finding
codes are owned by the adapter and use `your-adapter::local-name`.

Request only capabilities used by the package:

| Capability | Allows |
| --- | --- |
| `raw_graph_read` | select scanner-neutral raw nodes; required for every package |
| `snapshot_artifact_text_read` | resolve an `evidence_value` through an exact snapshot-bound artifact/path/pointer |
| `emit_semantic_nodes` | use node mappings |
| `emit_semantic_edges` | use edge mappings |
| `emit_findings` | use semantic policy rules |
| `emit_projections` | declare named projections |

These are interpreter operations, not host permissions. None permits shell,
process, native library, arbitrary filesystem, network, Python, or Rust code
execution.

## Step 3 — Build conservative detection

Detection decides whether a package is eligible for a raw graph; it does not
install or silently select the package. Each rule selects raw nodes and has:

- a positive `weight`;
- `minimum_matches` greater than zero; and
- `required: true` when absence must make the adapter ineligible.

At least one rule must be required. Prefer distinctive schema/container paths
and domain identifiers over broad rules such as “any JSON object.” Optional
signals can raise confidence, but they must not compensate for a missing
required signal.

Detection output includes each rule's count and a deterministic bounded sample
of matching raw node hashes, so a reviewer can understand why it matched.

## Step 4 — Map raw nodes and edges

Each node mapping combines a raw selector, a namespaced semantic type, a stable
identity source, and bounded attributes. Identity sources are:

```text
raw_node_id | identifier | label | source_location
```

Attribute sources are:

```text
identifier | label | relative_path | pointer | evidence_value
raw_property { key } | literal { value }
```

Mark an attribute `required: true` only when the semantic node is invalid
without it. A missing required value prevents that mapping; an optional missing
value is omitted.

An edge mapping translates a raw edge kind only when mapped source and target
semantic nodes exist. Use `from_semantic_type` and `to_semantic_type` whenever
one raw edge kind could connect several mapped types.

`evidence_value` is intentionally expensive and narrow. It can read only the
artifact hash and relative path already bound to the raw node and present in
the exact frozen snapshot. The interpreter performs bounded reads, verifies the
content hash, parses supported structured text without execution, and follows
the existing pointer. It cannot discover or open another path.

## Step 5 — Express audit policy

Milestone 4 supports six deterministic assertions:

| Assertion | Meaning |
| --- | --- |
| `minimum_type_count` | require at least N nodes of one semantic type |
| `maximum_type_count` | allow at most N nodes of one semantic type |
| `required_attribute` | require an attribute on every node of a type |
| `unique_external_id` | reject repeated external IDs within a type |
| `required_outgoing_relation` | require each node of a type to have a named outgoing relationship |
| `forbid_self_relation` | flag a named relationship from a node to itself |

Every rule needs a severity, concise title, reviewer-facing description, and
concrete remediation. The emitted finding code becomes
`your-adapter::your-rule-id` and retains source evidence.

These assertions test the semantic graph you defined. They do not establish
domain truth. Clinical and scientific policies must be independently reviewed
and tested against external reference cases.

## Step 6 — Publish projections

A projection is a named filtered view, not another analysis engine. List the
semantic types and relationship types needed by a user-facing task. The
resolved catalog contains exact semantic node/edge hashes and explains why a
projection is unavailable when required content is absent.

Examples include `pathex::cbc-decision-flow` or `oncova::guard-network`. Keep
projections small and purpose-specific rather than creating one
undifferentiated view of every mapped item.

## Step 7 — Add positive and negative fixtures

Every package requires both:

- at least one positive fixture that is detection-eligible; and
- at least one negative fixture that is not eligible.

Fixtures contain synthetic raw nodes and edges, never confidential project
data. For each fixture declare exact expectations:

```text
detection eligibility
minimum confidence (0..10,000 basis points)
semantic node counts by type
semantic relationship counts by type
exact finding-code set
exact projection availability by projection ID
```

Coverage is mandatory: every detection rule must match a fixture; every node
and edge mapping must emit; every policy must produce its finding in some
fixture; and every projection must be exercised. This prevents dead declarations
from passing simply because no fixture reached them.

## Step 8 — Check, install, detect, and approve

```bash
qbm adapter package check your-adapter.json
qbm adapter package add your-adapter.json
qbm adapter package list

qbm adapter detection run <run-id> <approved-raw-graph-hash>
qbm run approve <run-id> --stage adapter-detection \
  --expected-hash <detection-hash> --actor <reviewer>

qbm adapter plan create <run-id> <detection-hash> \
  --package-hash <exact-package-hash>
qbm run approve <run-id> --stage adapter-plan \
  --expected-hash <plan-hash> --actor <reviewer>

qbm semantic graph build <run-id> <plan-hash>
qbm run approve <run-id> --stage semantic-graph \
  --expected-hash <semantic-graph-hash> --actor <reviewer>

qbm semantic audit run <run-id> <semantic-graph-hash>
qbm run approve <run-id> --stage semantic-audit \
  --expected-hash <semantic-audit-hash> --actor <reviewer>

qbm projection catalog build <run-id> \
  <semantic-graph-hash> <semantic-audit-hash>
```

Review detection evidence, mapped counts, findings, and unavailable-projection
reasons before each approval. Automation may prepare every stage, but it does
not bypass these decisions.

## Versioning rules

- Never change content under an existing adapter ID/version. The registry
  rejects a different canonical hash for that pair.
- Increase the package version whenever detection, mapping, policy, projection,
  fixture, or manifest behavior changes.
- Treat the canonical package hash as the exact behavioral lock and the source
  hash as provenance for the imported bytes.
- A platform conformance-suite revision requires explicit re-certification of
  an unchanged package. It does not rewrite the original source provenance.
- Preserve regression fixtures from earlier bugs and add a negative fixture
  for every false-positive detection pattern discovered in real projects.

## Current boundary

Adapters interpret already-frozen evidence. They do not fetch APIs, watch
folders, ingest local bulk datasets, run GNN/CNN/neural engines, call models, or
perform optimization themselves. Connectors must first turn API or local-bulk
data into immutable snapshots. Any future behavior runner must execute behind
a separate isolation and approval boundary.

After adapter stages, the browser profiler may produce a reviewable
`qbm.benchmark-profile/v1`. The pure-Rust benchmark layer—not the adapter—uses
that approved profile for version comparison, deterministic classical
baselines, and QUBO/Ising construction. When no adapter is eligible, a valid
explicit `qbm.profile` can still enter the same optimization path. See
[`BIOMEDICAL_WORKFLOW.md`](BIOMEDICAL_WORKFLOW.md).

For implementation and measured verification details, see
[`MILESTONE_4.md`](MILESTONE_4.md).
