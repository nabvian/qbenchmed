# Project Graphs, Snapshots, Adapters, and Connectors

## 1. Canonical data hierarchy

Q-BenchMed stores six related but non-interchangeable objects:

```text
Project
  -> SourceSnapshot
       -> Inventory
       -> RawProjectGraph
       -> SemanticGraph
       -> BenchmarkProjection(s)
       -> AuditRun(s)
```

- A `Project` is stable identity and governance configuration.
- A `SourceSnapshot` is immutable source evidence.
- An `Inventory` describes included and excluded artifacts.
- A `RawProjectGraph` contains mechanically observed facts.
- A `SemanticGraph` contains approved interpretations.
- A `BenchmarkProjection` contains one explicit mathematical or evaluation question.
- An `AuditRun` binds all versions, approvals, findings, and results.

## 2. Identity and version rules

Every stored object has:

- a human-readable ID;
- a schema version;
- a content hash;
- a creation timestamp;
- the producing component and version;
- parent artifact hashes;
- a sensitivity classification;
- an approval state.

Content identity uses canonical serialization. Map keys are ordered, unstable timestamps are excluded from the identity payload, paths are normalized relative to the snapshot root, and platform-specific separators are converted to `/`.

Recommended identity shape:

```text
urn:qbm:<artifact-kind>:<sha256>
```

Human versions such as `2.4.0` are metadata. They never replace a content hash.

## 3. Project manifest

Projects may include `.qbench/project.yaml`. Absence of this file does not prevent discovery, but its presence makes onboarding safer and more deterministic.

```yaml
schema_version: qbm.project/v1

project:
  id: ONCOVA
  display_name: ONCOVA
  domain: oncology
  version_source: git
  sensitivity: restricted
  default_adapter: oncova

source_policy:
  include:
    - "**/*.yaml"
    - "**/*.json"
    - "**/*.rs"
    - "**/*.py"
    - "**/*.md"
  exclude:
    - ".git/**"
    - "**/.env"
    - "patient-data/**"
    - "secrets/**"
    - "checkpoints/raw/**"
  follow_symlinks: false
  maximum_single_file_bytes: 1073741824
  large_artifact_mode: manifest_only

adapters:
  - oncova
  - rust-software

policies:
  - generic-structure/v1
  - provenance/v1
  - clinical-research-boundary/v1
  - model-lineage/v1

behavior_runners:
  - id: guard-gates
    command_ref: oncova.guard-gates/v1
    network: denied
    fixture_set: guard-golden/v1

projections:
  - guard-reachability
  - corpus-scope-coverage
  - retrieval-regression
  - execution-dag

approval_profile: regulated-onboarding/v1
```

The manifest is configuration, not trusted authority. Its hashes, paths, commands, and policies are validated during intake.

## 4. Source snapshot schema

A `SourceSnapshot` records the exact inputs to every later stage.

```yaml
schema_version: qbm.snapshot/v1
snapshot_id: urn:qbm:snapshot:<sha256>
project_id: ONCOVA
source_version: "0.1.0"
git:
  commit: "<commit-or-null>"
  dirty: false
inventory_hash: "<sha256>"
inclusion_policy_hash: "<sha256>"
artifacts:
  - artifact_id: source:guard-manifest
    kind: file
    relative_path: GUARD/bundle/BUNDLE_MANIFEST.yaml
    size_bytes: 12345
    sha256: "<sha256>"
    media_type: application/yaml
    sensitivity: internal
connectors:
  - receipt_id: oncova-corpus-2026-08
    connector_kind: local_bulk
    upstream_version: "2026-08"
    receipt_hash: "<sha256>"
    mirror_manifest_hash: "<sha256>"
status: discovered
```

Snapshots use status pointers, not mutable status fields inside the content-addressed object. Approval records say that a specific snapshot hash is validated or approved.

## 5. Source artifact contract

All connectors normalize their outputs to the same contract:

```yaml
schema_version: qbm.source-artifact/v1
source_id: clinical-trials-delta
mode: api_delta
upstream_uri: "redacted-or-public-identifier"
upstream_version: "api-v2"
query_hash: "<sha256>"
acquired_at: "2026-08-30T12:00:00Z"
local_artifact: mirror/clinical-trials/2026-08-30.jsonl
sha256: "<sha256>"
media_type: application/x-ndjson
license:
  id: public-data
  status: verified
privacy:
  classification: public
validation:
  schema: clinical-trials-v2
  record_count: 142
  accepted_count: 139
  quarantined_count: 3
```

Secrets, authorization headers, cookies, and raw private query parameters are never stored in this object.

## 6. Connector model

Connectors acquire and mirror data. They do not interpret domain meaning and do not write directly into an approved graph.

### 6.1 Required connector types

#### Local folder connector

- walks approved roots;
- applies include/exclude policies;
- streams hashes;
- detects changes relative to a prior receipt;
- never follows symlinks by default.

#### Archive connector

- supports approved ZIP/TAR formats;
- rejects absolute paths and `..` traversal;
- enforces expanded-size, file-count, and compression-ratio limits;
- extracts into quarantine only.

#### Bulk connector

- handles large local releases without copying them unnecessarily;
- verifies per-file or manifest hashes;
- records upstream release and licence;
- supports incremental parsing checkpoints;
- emits one immutable receipt.

#### API-delta connector

- records endpoint identity, API version, normalized query hash, cursor, rate-limit behavior, and response artifact hashes;
- writes responses to a local mirror before parsing;
- supports retry with idempotency and backoff;
- stages additions, modifications, removals, and tombstones;
- never places a live response directly into an approved audit or decision path.

#### Database snapshot connector

- uses a read-only account;
- exports schema separately from permitted data summaries;
- records database engine/version and transaction snapshot identity;
- supports metadata-only mode;
- prohibits uncontrolled row export.

#### Runtime API contract connector

- imports OpenAPI/GraphQL/protobuf contracts;
- captures approved, de-identified request/response fixtures;
- supports local record/replay;
- measures contract drift, failure behavior, and latency separately from knowledge refresh.

#### Model artifact connector

- records model format, hash, input/output signature, preprocessing/postprocessing IDs, training dataset version, evaluation dataset version, and framework metadata;
- keeps weights outside the graph;
- permits operator-level inspection only when the format and policy allow it.

#### Git connector

- records repository identity, commit, tree hash, branch/ref metadata, submodule state, and dirty status;
- never performs a network clone without an approved network stage.

### 6.2 Connector lifecycle

```text
DISCOVER
  -> ACQUIRE TO QUARANTINE
  -> VERIFY TRANSPORT
  -> VERIFY HASH
  -> CHECK LICENCE/PRIVACY
  -> NORMALIZE RECEIPT
  -> MIRROR COMMIT
  -> SNAPSHOT CANDIDATE
  -> APPROVAL
```

If an API is unavailable, the latest approved snapshot remains usable. Failure to refresh is reported; it does not mutate previous results.

## 7. Raw Project Graph

The raw graph answers “what was observed?” It does not answer “what does it mean?”

### 7.1 Raw node examples

- file, directory, archive member;
- YAML/JSON object and field;
- Rust crate, module, type, trait, function, call site;
- Python module, class, function, import;
- API operation, request schema, response schema;
- database table, column, key, view;
- model, tensor, operator, input signature, output signature;
- dataset, split, metric, test fixture;
- document section, citation, evidence span;
- build target, dependency, feature flag, test command.

### 7.2 Raw edge examples

- `contains`;
- `imports`;
- `calls`;
- `references`;
- `depends_on`;
- `reads`;
- `writes`;
- `declares`;
- `uses_schema`;
- `feeds_tensor`;
- `produces`;
- `tested_by`;
- `documented_by`;
- `sourced_from`.

Every raw node and edge contains a provenance list:

```yaml
provenance:
  - snapshot_id: urn:qbm:snapshot:<sha256>
    artifact_id: source:rules
    relative_path: knowledge/rules.yaml
    byte_start: 1180
    byte_end: 1294
    line_start: 42
    line_end: 47
    scanner: yaml/v1
    extraction_confidence: 1.0
```

## 8. Approved Semantic Graph

The semantic graph applies approved mappings to raw facts.

### 8.1 Core semantic node types

- `ProjectComponent`;
- `SourceArtifact`;
- `Dataset`;
- `Schema`;
- `Input`;
- `Context`;
- `DerivedFeature`;
- `Rule`;
- `ConditionGroup`;
- `Constraint`;
- `SafetyGate`;
- `Target`;
- `Outcome`;
- `EvidenceClaim`;
- `Intervention`;
- `Model`;
- `Tensor`;
- `PipelineStage`;
- `Metric`;
- `TestCase`;
- `ReportElement`.

Adapters may add namespaced node kinds, but generic engines may ignore unknown kinds safely.

### 8.2 Semantic edge types

- `requires_all`;
- `requires_any`;
- `derives_from`;
- `evaluates`;
- `produces`;
- `blocks`;
- `overrides`;
- `precedes`;
- `routes_to`;
- `supports`;
- `contradicts`;
- `scoped_to`;
- `trained_on`;
- `validated_by`;
- `supersedes`;
- `equivalent_to`;
- `maps_to`;
- `must_preserve`.

### 8.3 Do not flatten conjunctions

This logic:

```text
(HGB AND MCV AND FERRITIN) OR (HGB AND STFR)
```

must be represented as two condition groups:

```text
ConditionGroup A --requires_all--> HGB, MCV, FERRITIN
ConditionGroup B --requires_all--> HGB, STFR
A --enables--> outcome
B --enables--> outcome
```

A simple edge from every input directly to the outcome would incorrectly convert the rule into an OR relation. The adapter conformance suite must include conjunction-preservation tests.

### 8.4 Semantic approval metadata

Every interpretation records:

- mapping rule ID and version;
- adapter and version;
- raw evidence IDs;
- confidence;
- approval status;
- approver and approval event;
- sensitivity and export restrictions.

Unapproved semantics may appear in a discovery report but cannot enter an approved optimization projection.

## 9. Benchmark projection

A projection is a versioned contract, not an informal matrix.

```yaml
schema_version: qbm.projection/v1
projection_id: oncova.guard-minimum-tests/v1
semantic_graph_hash: "<sha256>"
purpose: research

decision_variables:
  node_selector:
    kind: Input
    attributes:
      selectable: true

targets:
  node_selector:
    kind: SafetyGate
    attributes:
      required: true

relations:
  path_semantics: evaluability
  preserve_conjunctions: true

objective:
  kind: minimum_cost_full_coverage

constraints:
  - kind: preserve_all_targets
  - kind: exclude_sensitive_context

evaluation:
  fixture_set: oncova-guard-golden/v1
  minimum_behavior_agreement: 1.0

allowed_backends:
  - exact_ilp
  - greedy_check
  - simulated_annealing

quantum_export: forbidden
```

Projection builders fail closed if requested semantics are absent or if a chosen backend cannot encode required relations exactly.

## 10. Adapter interface

An adapter has five responsibilities:

1. detect whether it applies;
2. validate source preconditions;
3. map raw facts to semantic nodes, edges, and constraints;
4. provide project policies and behavior-runner contracts;
5. build named benchmark projections.

Conceptual Rust contract:

```rust
pub trait ProjectAdapter: Send + Sync {
    fn descriptor(&self) -> AdapterDescriptor;
    fn detect(&self, inventory: &Inventory, raw: &RawProjectGraph)
        -> Result<Detection, AdapterError>;
    fn validate_source(&self, ctx: &AdapterContext)
        -> Result<Vec<Finding>, AdapterError>;
    fn build_semantic_graph(&self, ctx: &AdapterContext)
        -> Result<SemanticGraphDraft, AdapterError>;
    fn policies(&self) -> Vec<PolicyRef>;
    fn projection_catalog(&self) -> Vec<ProjectionDescriptor>;
    fn build_projection(&self, request: ProjectionRequest, ctx: &AdapterContext)
        -> Result<BenchmarkProjection, AdapterError>;
}
```

An adapter returns data. It does not call the database directly, write reports, manage approvals, or choose algorithms.

## 11. Adapter packaging

Use three trust levels:

### Built-in Rust adapters

Compiled into the official distribution. They have full access only through restricted service interfaces and are covered by the main test suite.

### WASM/WASI adapters

Preferred third-party plugin format. Capabilities such as reading approved snapshot artifacts are explicitly granted. Network and arbitrary filesystem access are denied.

### External process adapters

Used for ecosystems that require Python or another runtime. Communication uses a versioned JSON-lines protocol over standard input/output. The process receives a staged, restricted view and no secrets unless its capability contract is approved.

Do not expose Rust dynamic libraries as the stable third-party ABI.

## 12. Adapter detection

Detection evidence may include:

- `.qbench/project.yaml` declaration;
- required paths and manifest IDs;
- schema fingerprints;
- known rule structures;
- Cargo/Python package metadata;
- model signatures;
- documentation markers.

Detection result:

```yaml
adapter: oncova
confidence: 0.98
reasons:
  - GUARD/bundle/BUNDLE_MANIFEST.yaml declares ONCOVA-GUARD
  - CORPUS/schemas/claim.schema.json is present
conflicts: []
requires_approval: true
```

Confidence never substitutes for approval during first onboarding.

## 13. PATHEX adapter

The PATHEX adapter maps:

```text
observed/orderable tests -> Input(selectable=true)
patient context          -> Context(selectable=false)
calculated values        -> DerivedFeature
AND/OR arms              -> ConditionGroup
rule conditions          -> Rule
safety/clinical outputs  -> Outcome or SafetyGate
knowledge citation       -> EvidenceClaim/SourceArtifact
```

Required policies include:

- derived values cannot become orderable inputs;
- context cannot be charged as a selectable test;
- units and normalization contracts must agree;
- every clinical threshold needs provenance;
- safety outputs cannot silently disappear;
- deprecated outcomes require status and migration evidence;
- patient records cannot enter benchmark artifacts.

Initial projections:

- input-to-outcome evaluability;
- minimum panel at coverage floor;
- input marginal value and redundancy;
- rule reachability;
- missing-data resilience;
- behavior preservation across controlled scenarios;
- version regression.

## 14. ONCOVA adapter

The ONCOVA adapter creates separate semantic subgraphs.

### 14.1 GUARD subgraph

```text
context/observations
  -> derived states
  -> composed features
  -> validators and kill gates
  -> required tests, urgent flags, conflicts, board questions
```

Audits include reachability, precedence, missing references, safety-path coverage, unmapped findings, and runner parity. Optimization is limited to approved research questions such as minimum evidence/test sets that preserve every required safety path.

### 14.2 CORPUS subgraph

```text
source artifact
  -> chunk
  -> evidence claim
  -> scope axes
  -> management direction / contraindication / required evidence
```

Audits include source hashes, licence status, review state, scope completeness, supersession, provenance spans, and required veto coverage. Machine-proposed claims remain machine-proposed; the adapter cannot promote them.

### 14.3 Retrieval projection

Uses approved query fixtures and expected relevant claims/chunks. Metrics include recall at K, precision, wrong-scope retrievals, critical misses, index-versus-scan parity, latency, and change across corpus snapshots.

### 14.4 Execution graph projection

Checks topological order, missing dependencies, cycle freedom, halt semantics, validators preceding routing, declared stages without tests, and failure propagation.

### 14.5 Research graph projection

Drug, target, disease, pathway, assay, and evidence relationships may be projected into explicitly bounded research objectives. This graph is firewalled from GUARD clinical/safety outputs.

### 14.6 Source refresh

ONCOVA bulk, API delta, and deep-fetch artifacts must be mirrored, hashed, validated, and folded into a new corpus snapshot before the adapter consumes them. Q-BenchMed never audits a live API response as though it were an approved corpus.

## 15. ML, GNN, and CNN adapter

The model adapter records:

- model hash and format;
- tensor names, shapes, and dtypes;
- preprocessing and postprocessing graph;
- labels or task schema;
- training/evaluation dataset identities;
- feature schema and node/edge schema for GNNs;
- metrics and acceptance thresholds;
- hardware/runtime requirements;
- model-card and licence references.

Weights, images, patient records, and training corpora remain external unless an explicitly approved evaluation runner needs controlled access.

Projections include:

- data lineage;
- signature compatibility;
- unused input features;
- output-label drift;
- preprocessing changes;
- evaluation slice regressions;
- feature-selection or test-selection objectives;
- model-versus-rule disagreement.

## 16. Adapter onboarding lifecycle

```text
1. Auto-inventory the project
2. Produce raw graphs
3. Select or create an adapter mapping pack
4. Review semantic classifications
5. Review exclusions and sensitivity
6. Define policy pack
7. Define behavior runners and golden fixtures
8. Define safe projections/objectives
9. Approve golden semantic snapshot
10. Enable continuous automatic discovery and diff
```

The platform should generate a draft mapping file and a list of unresolved objects. A human resolves ambiguity once. Subsequent snapshots reuse the mapping and surface only new or conflicting entities.

## 17. Schema evolution

- All public artifacts carry `schema_version`.
- Readers support the current version and a documented compatibility window.
- Migrations are pure transformations that preserve the original artifact.
- Unknown fields are retained where possible.
- Unknown semantic kinds do not crash generic audits.
- Removing or changing field meaning requires a major schema version.
- Adapter compatibility declares supported raw, semantic, and projection schema ranges.

