# Milestone 4 — Declarative Adapter SDK

> Historical completion record: this page describes the adapter-SDK milestone.
> The current profile, comparison, optimization, and QUBO/Ising flow is
> documented in [Biomedical Workflow and Architecture](BIOMEDICAL_WORKFLOW.md).

Status: complete (2026-09-01)  
Implementation language: Rust  
Third-party adapter format: strict, data-only JSON  
Imported-code execution: none

This completion record describes the implemented contracts, security boundary,
measured tests, and a real approval-gated CLI run. Milestone 4 is the reusable
adapter foundation; it does not claim that production PATHEX, ONCOVA, or other
biomedical adapter packages already exist.

## Purpose

Milestone 3 creates a domain-blind raw graph. Milestone 4 lets an adapter add a
reviewable interpretation without giving that adapter permission to execute
code.

An adapter can declare:

- how to recognize a compatible project;
- how matching raw nodes and edges become namespaced semantic objects;
- which semantic assertions produce audit findings;
- which named semantic views should be available to users; and
- fixtures that prove the declared behavior before installation.

The trusted Rust interpreter performs every operation. An adapter package
cannot load a dynamic library, run a process, invoke a build script, use the
network, compile imported Rust, or directly read arbitrary files.

## Delivered architecture in the current source tree

```text
approved raw-graph hash
  -> detect every installed, conformant package
  -> adapter-detection hash
  -> manual approval
  -> select exact eligible package hashes
  -> adapter-plan hash
  -> manual approval
  -> project approved raw graph + frozen snapshot
  -> semantic-graph hash
  -> manual approval
  -> evaluate adapter policy packs
  -> semantic-audit hash
  -> manual approval
  -> resolve named projections
  -> projection-catalog hash
  -> optional final approval / downstream consumption
```

The generic Milestone 3 audit stays available as a parallel, domain-blind
report. An adapter does not replace or rewrite it.

### Workspace components

```text
qbm-domain    versioned adapter, semantic graph, policy, finding and projection contracts
qbm-adapter   strict JSON decoder, static validator, fixture kit and bounded interpreter
qbm-store     schema-v4 immutable manifests and run bindings
qbm-app       approval-gated orchestration over exact predecessor hashes
qbm-cli       package, detection, plan, semantic audit and projection commands
```

## Adapter package contract

The accepted top-level schema is `qbm.adapter-package/v1`; the interpreter API
is `qbm.adapter-api/v1`. Unknown fields and duplicate JSON keys are rejected.
The canonical package hash is independent of object-key order, while the hash
of the originally imported bytes remains stored as separate supply-chain
evidence.

One package contains six sections:

1. `manifest` — portable ID, version, display metadata, licence, minimum domain
   schema and requested declarative capabilities.
2. `detection` — weighted selectors over scanner-neutral raw nodes.
3. `mappings` — raw-node and raw-edge rules that emit semantic nodes and
   relationships.
4. `policies` — deterministic assertions evaluated over that package's
   semantic output.
5. `projections` — named filtered views over semantic types and relationships.
6. `fixtures` — synthetic raw graph fragments with exact expected detection,
   mapping, finding and projection results.

An example exercising all six sections is available at
[`examples/adapters/generic-manifest-v1.json`](../examples/adapters/generic-manifest-v1.json).
The step-by-step public authoring workflow is documented in
[`ADAPTER_AUTHORING.md`](ADAPTER_AUTHORING.md).

## Capabilities and isolation

Capabilities are declarations validated against package contents:

```text
raw_graph_read
snapshot_artifact_text_read
emit_semantic_nodes
emit_semantic_edges
emit_findings
emit_projections
```

Every package must request `raw_graph_read`. Node, edge, policy, projection and
evidence-value declarations require their corresponding capabilities.
Capabilities do not expose host functions or arbitrary I/O; they tell the
trusted interpreter which declarative operations the package is allowed to ask
for.

`snapshot_artifact_text_read` is deliberately narrow. An `evidence_value`
mapping may resolve only the artifact hash, normalized relative path and syntax
pointer already present on a raw node, and only when that exact artifact/path
pair belongs to the raw graph's frozen source snapshot. Unsupported Rust or
unknown documents do not become executable inputs.

## Detection

A selector can restrict source format, raw node kind, path prefix/suffix,
label, identifier, identifier presence and generic definition status. Each
detection rule has a positive integer weight, a minimum match count and a
`required` flag.

For each exact installed package version, detection returns:

- match count and result for every rule;
- matched and maximum weight;
- eligibility after required rules; and
- confidence as an integer from 0 to 10,000 basis points.

Detection does not silently choose an adapter. A reviewer approves the report,
then a plan explicitly locks one eligible package hash per adapter ID.

## Semantic mapping

Node mappings select raw nodes, assign an adapter-namespaced semantic type,
choose a stable external identity and bind attributes from controlled sources:

```text
identifier        raw identifier
label             raw display label
relative_path     normalized frozen source path
pointer           scanner syntax/JSON pointer
evidence_value    bounded scalar/structured value from exact frozen evidence
raw_property      one scanner-neutral property by declared key
literal           package-declared JSON constant
```

Edge mappings translate selected raw edge kinds only when mapped source and
target semantic nodes exist. Optional source/target semantic-type filters avoid
cross-product ambiguity.

Semantic types and relationships must use the adapter namespace:

```text
<adapter-id>::<portable-name>
```

Every emitted node and edge records the adapter ID, exact package hash, mapping
rule ID, originating raw identity and immutable evidence. The semantic graph
also embeds the complete adapter locks from the approved plan.

## Semantic policies

The current declarative assertion set is:

```text
minimum_type_count
maximum_type_count
required_attribute
unique_external_id
required_outgoing_relation
forbid_self_relation
```

Each failed assertion produces a canonical, evidence-linked `AuditFinding`
whose code is `<adapter-id>::<policy-rule-id>`. Findings include severity,
title, concrete message, remediation, affected semantic nodes and immutable
source evidence. A summary counts informational, warning, error and critical
findings.

## Projection catalog

A projection declares a package-local ID plus included semantic and relationship
types. Resolution records exact node and edge hashes. It is marked unavailable,
with deterministic reasons, when a declared type or relationship is absent.

The projection catalog requires matching, approved semantic-graph and
semantic-audit hashes. Therefore a user cannot view a projection as audited
when it was resolved from different semantic evidence.

## Conformance and installation

`adapter package check` strictly decodes a package, calculates its canonical
hash, runs static validation and executes its self-contained fixtures. It saves
the pass/fail conformance certificate but does not install the package.

Static validation covers schema/API compatibility, portable and unique IDs,
namespace ownership, capability consistency, known mapping/policy/projection
references, safe fixture paths and hard resource ceilings. Fixtures compare
exact semantic type counts, relationship counts, finding-code sets and
projection availability, plus detection eligibility and a minimum confidence.

`adapter package add` installs only a passing package. The registry record locks:

```text
canonical package hash
original JSON artifact hash
digest-specific conformance report hash
validated package data
installation timestamp (excluded from identity)
```

The interpreter revalidates the source artifact, canonical package hash and
conformance identity when a registry record is used. A reused adapter
ID/version with different content is rejected.

The conformance schema is also a frozen behavioral suite revision. Any change
to validation, fixture interpretation, or pass/fail semantics must bump that
revision. An identical immutable package can then be explicitly re-certified;
the registry rotates only its certificate pointer, while retaining the original
source hash and installation timestamp. Older certificates and record manifests
remain in content-addressed storage.

Conformance proves internal consistency against publisher-supplied synthetic
fixtures and platform safety rules. It does **not** prove clinical validity,
financial correctness, scientific truth, regulatory approval, publisher
identity, or package authenticity. Those require independent domain review,
curated external evaluation datasets, and a future signing/trust layer. A
passing package may still encode a confidently wrong domain policy.

## Resource boundaries

Default host-controlled limits include:

```text
package JSON:                       2 MiB
retained matches per detection rule: 16
registry packages per detection:   256
total detection output:             64 MiB
selected adapters per plan:        16
mapping rules per package:         1,024
semantic nodes per projection:     250,000
semantic edges per projection:     500,000
semantic findings per audit:       250,000
affected nodes/evidence per finding: 4,096 each
total semantic output:             128 MiB
total audit output:                 64 MiB
projection memberships:           500,000
total projection output:          128 MiB
one evidence artifact:             2 MiB
cumulative evidence input:         64 MiB
deterministic rule evaluations:     50,000,000
```

Additional limits bound detection rules, policy rules, projections, fixtures,
fixture graph size, attributes, nesting depth and total semantic output bytes.
Limit exhaustion fails the operation; partial output is not accepted as a
complete semantic result.

## Persistence and approval invariants

Database schema version 4 adds immutable conformance, adapter registry,
detection, plan, semantic graph, semantic report and projection catalog
manifests, plus their run bindings.

The application layer enforces these exact stages:

| Producer | Required approved predecessor | New stage |
| --- | --- | --- |
| adapter detection | `raw-graph` | `adapter-detection` |
| adapter plan | `adapter-detection` | `adapter-plan` |
| semantic graph | `adapter-plan` | `semantic-graph` |
| semantic audit | `semantic-graph` | `semantic-audit` |
| projection catalog | matching `semantic-graph` and `semantic-audit` | `projection-catalog` |

Each stage output is attached to the same run. Recomputing a changed output
places that stage back into `waiting_approval`; an approval for an older hash
does not authorize the new one.

The generic `stage complete` command cannot write any built-in platform stage,
including `projection-catalog`; only the typed producer that validates its
predecessor and run binding can create those outputs. Adapter registry listing
also performs a database-only cardinality preflight before loading package
manifests, so the interpreter's registry limit is enforced before allocation.

## CLI workflow

First check and install a data-only package:

```bash
qbm adapter package check examples/adapters/generic-manifest-v1.json
qbm adapter package add examples/adapters/generic-manifest-v1.json
qbm adapter package list
```

After an existing run has an approved raw graph:

```bash
qbm adapter detection run <run-id> <raw-graph-hash>
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

Every command emits structured JSON. `show` and `list` subcommands are present
for packages, detections, plans, semantic graphs, semantic reports and
projection catalogs; conformance certificates support `show`.

## Multi-project and multi-adapter use

The platform never copies all source YAML, JSON, Rust, neural-model or dataset
content into a package. A project is frozen and scanned once. Biomedical
packages then select only the raw evidence they understand.

PATHEX, ONCOVA, and other biomedical adapters can use the same SDK while owning
separate namespaces. One approved plan may select multiple adapter IDs, but
only one exact version of each ID. Their semantic outputs coexist in one
immutable graph and remain attributable to their package hashes.

API and local-bulk connectors are intentionally outside this milestone. They
must first create immutable, provenance-rich snapshots; adapters can then
interpret those snapshots through this same deterministic path.

## Measured verification

The final offline release checks passed on 2026-09-01 (Asia/Kolkata):

```text
cargo fmt --all -- --check                                      PASS
cargo clippy --workspace --all-targets --offline -- -D warnings PASS
cargo test --workspace --offline                                PASS (60/60)
cargo deny check                                                PASS
```

The 60 tests include strict duplicate/unknown/trailing JSON rejection; positive
and negative fixtures; namespace and capability checks; exact source provenance;
snapshot evidence containment; evidence, package, fuel, registry and cumulative
output ceilings; bounded detection evidence; duplicate-selection rejection;
deterministic identities at every adapter stage; schema-v4 migration/reopen;
certificate rotation; immutable run bindings; fabricated built-in-stage
rejection; every predecessor approval gate; and event-chain verification.

One fresh, real CLI run imported
[`examples/projects/generic-manifest/rules.json`](../examples/projects/generic-manifest/rules.json),
installed the example adapter, and approved every material stage:

| Object | Measured identity/result |
| --- | --- |
| run | `65c6d492-4c8c-4a16-875b-75b7a948b453` |
| inventory hash | `6ce14d9f109f1cd3e5778aabcfe31ed617936ae244290d26d512e1ab7cf03ee2` |
| snapshot | `7ad322251fb81bfa740ea1e7ff98eabe6930239e00a261e2157981484acff37c` |
| raw graph | `4fe076667f748c3c6d018d93bd90f6458042bd1d0a46529c68c902870bf93760` (7 nodes, 7 edges) |
| generic audit | `5c1641981abe186cbbba136105a7c77b99861392b7d1e25cb6ae40c0d2edbcc5` |
| adapter package | `58004d7194bd82635abc886ea4dcae13a5509436f82d55cca2e65c4b59cd0c26` |
| conformance | `68d9b07878d4dce4df328ad4d58f94d4b6a29cea2e577cf5760939e6dacf627c` (102 checks, 0 issues) |
| detection | `4eb292e1f7200f71bf832175864572701c71ba682dd2b831c1553e235b5e30a8` |
| adapter plan | `c798b12b87ffbd2bd8119f3df2193f723bfd3f3eca9d2bbca16641892a1100b3` |
| semantic graph | `460c2ff92a7fe4857b52ea124c6d7228a6e25a3cdd2e7e1b0691d389e78026bf` (1 node, 1 edge) |
| semantic audit | `1332004f9e0738745cba341ede8d5bde3a1d7cf687309ce64a7b6dff991ab718` (1 warning; 0 errors/critical) |
| projection catalog | `f0f32b0c39852194b779e554e0e11360bf2c08480099a5e86bb9b5f6908a138a` |
| `definition-network` | available (1 node, 1 edge) |
| approvals / event chain | 9 exact-hash approvals / valid |

As a negative smoke check, attempting generic completion of the protected
`projection-catalog` stage exited with status 2 and made no stage mutation.
Dependency policy found no rejected advisory, licence, or source; duplicate
transitive versions of `hashbrown` and `syn` remain warnings, not failures.

## Accepted post-Milestone-4 hardening work

The release boundary is a local, single-user CLI/application service. The final
security review found no High/Critical blocker inside that boundary. It did
retain these later hardening items:

- make low-level public `PlatformStore::save_*` APIs independently recompute
  more caller-supplied identities instead of relying on the guarded app/engine;
- authenticate and sign reviewer/publisher identities rather than treating
  local actor IDs and hash chains as identity assertions;
- replace registry count preflight plus bounded loading with a transactional,
  streaming/paginated registry interface.

These become release requirements before a browser-based multi-user or remote
deployment claims an adversarial tenant boundary.

## Deliberately not claimed by this milestone

- executable third-party plugins or native dynamic libraries;
- arbitrary filesystem, process or network access from adapters;
- PATHEX- or ONCOVA-specific production mapping packs;
- API-delta or local-bulk acquisition connectors;
- behavioral execution of imported projects or models;
- optimization, QUBO/Ising or quantum-provider execution;
- browser UI.

Those remain later roadmap work. Milestone 4 supplies the safe semantic adapter
foundation they depend on.
