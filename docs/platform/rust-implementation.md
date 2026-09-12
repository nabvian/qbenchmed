# Rust Implementation Map

## 1. Language policy

Rust is the default for every component whose ecosystem support is adequate:

- domain types and canonical serialization;
- CLI and local HTTP server;
- orchestration and approval state machine;
- file intake, archive handling, hashing, manifests, and snapshots;
- static scanners and parser coordination;
- graph storage and algorithms;
- adapter SDK and built-in adapters;
- policy evaluation and findings;
- version diff;
- report generation;
- classical heuristics, QUBO construction, Ising conversion, and local simulation;
- database and artifact-store access;
- web frontend compiled to WebAssembly;
- connectors, file watchers, concurrency, and job control;
- security checks and sandbox coordination.

Python is retained only where it has a clear ecosystem advantage or preserves validated existing work:

- current Q-BenchMed mathematical reference during migration;
- Qiskit and IBM quantum provider integration;
- PyTorch/TensorFlow-specific inspection that cannot be implemented safely from stable model formats;
- selected scientific evaluation libraries behind a controlled bridge.

Python is never the orchestrator, source of run truth, approval system, or artifact registry.

### Rust placement summary

| Platform area | Rust responsibility | Non-Rust exception |
|---|---|---|
| CLI | All commands, validation, formatting, local process control | None required |
| Browser backend | HTTP API, uploads, SSE, authentication, workers | None required |
| Browser frontend | Rust/WASM components and typed API client | Tiny browser interoperability shim if required |
| Intake/snapshots | Safe archive handling, inventory, streaming hashes, content store | External malware scanner may be invoked as a tool |
| Source scanning | Structured parsers, tree-sitter coordination, spans, raw graph | Python-framework inspection only when stable files are insufficient |
| Adapters/policies | SDK, built-in adapters, mapping engine, policy engine | Sandboxed WASM or process plugins for third parties |
| Audit/diff/reports | Entire implementation | Optional external PDF renderer |
| Behavior execution | Runner contracts, sandbox control, capture, metrics | The audited project's own approved runtime |
| Classical optimization | Native scoring, graph algorithms, exact small solver, ILP bridge, heuristics | External solver binary/library when selected |
| QUBO/Ising | Construction, validation, conversion, decode, resource estimates | None required after parity port |
| Quantum | Payload preparation, approval, result validation, native rescoring | Qiskit/IBM provider bridge in Python |
| ML/model | ONNX/signature/lineage inspection and evaluation orchestration | PyTorch/TensorFlow bridge for framework-specific operations |
| Persistence/security | SQLite/store, event log, approvals, capability checks, signing | OS sandbox/keychain services through system APIs |

## 2. Proposed repository layout

The current Python implementation should remain operational while the Rust platform is built beside it.

```text
qbenchmed/
├── Cargo.toml                       # Rust workspace
├── rust-toolchain.toml
├── crates/
│   ├── qbm-domain/                  # IDs, schemas, graphs, findings, objectives
│   ├── qbm-canonical/               # canonical JSON, hashing, signatures
│   ├── qbm-store/                   # metadata DB, event log, artifact store
│   ├── qbm-intake/                  # folder/archive/Git intake and quarantine
│   ├── qbm-snapshot/                # inventory, manifests, immutable snapshots
│   ├── qbm-scanner-sdk/             # scanner contracts
│   ├── qbm-scanners/                # YAML/JSON/Rust/Python/API/model scanners
│   ├── qbm-connector-sdk/           # acquisition contract
│   ├── qbm-connectors/              # local/bulk/API/DB/model/Git connectors
│   ├── qbm-adapter-sdk/             # adapter contracts and plugin protocol
│   ├── qbm-adapter-generic/
│   ├── qbm-adapter-pathex/
│   ├── qbm-adapter-oncova/
│   ├── qbm-adapter-ml/
│   ├── qbm-policy/                  # policy language and engine
│   ├── qbm-audit/                   # generic structural/provenance audits
│   ├── qbm-diff/                    # graph, behavior, schema, artifact diff
│   ├── qbm-runner/                  # isolated behavior-runner coordination
│   ├── qbm-objective/               # objective validation and projection
│   ├── qbm-optimizer/               # classical optimizer abstraction
│   ├── qbm-qubo/                    # QUBO/Ising construction and validation
│   ├── qbm-quantum-protocol/        # provider-neutral job/result contract
│   ├── qbm-report/                  # JSON/CSV/Markdown/HTML reports
│   ├── qbm-orchestrator/            # resumable stage state machine
│   └── qbm-app/                     # application services used by all clients
├── apps/
│   ├── qbm-cli/                     # CLI binary
│   ├── qbm-server/                  # Axum/Tokio local server and workers
│   └── qbm-ui/                      # Rust/WASM minimal UI
├── bridges/
│   ├── python-protocol/             # shared JSON schemas
│   ├── qbm-python-reference/        # wrapper around current qbm package
│   ├── qbm-qiskit/                  # local Qiskit and IBM bridge
│   └── qbm-ml-python/               # optional framework inspection/eval
├── adapters/                        # mapping packs and WASM/process plugins
├── policies/                        # versioned generic/domain policy packs
├── schemas/                         # published JSON Schemas
├── fixtures/                        # safe golden projects and graph fixtures
├── docs/platform/                   # this specification
├── qbm/                             # current Python reference, unchanged first
├── experiments/                     # current experiments
└── tests/                            # current Python tests
```

Do not move the existing Python code until Rust parity tests consume the same fixtures and reproduce its accepted results.

## 3. Crate boundaries

### `qbm-domain`

Pure domain types:

```rust
ProjectId, SnapshotId, ArtifactId, RunId, StageId
Inventory, SourceSnapshot, ConnectorReceipt
RawProjectGraph, SemanticGraph, BenchmarkProjection
Finding, EvidenceRef, PolicyResult
ObjectiveContract, CandidateSolution, RunRecord
ApprovalRequest, ApprovalDecision, ApprovalPolicy
```

Rules:

- no database code;
- no web framework types;
- no filesystem access;
- no provider SDK types;
- serializable with explicit schema versions;
- invalid states rejected by constructors where practical.

### `qbm-canonical`

- canonical JSON serialization;
- SHA-256 content identities;
- optional BLAKE3 for fast internal integrity checks while retaining SHA-256 interchange IDs;
- deterministic path and floating-point normalization;
- manifest signing and verification;
- hash-chain support for approval events.

### `qbm-store`

Repository interfaces and implementations for:

- project metadata;
- snapshots and pointers;
- runs and stage state;
- append-only events;
- approvals;
- findings;
- artifact metadata;
- content-addressed blobs.

Initial implementation: SQLite plus filesystem artifact storage. Domain services depend on repository traits, allowing PostgreSQL/object storage later.

### `qbm-intake`

- streamed multipart receive;
- safe archive inspection/extraction;
- local-path registration;
- quarantine lifecycle;
- file count/size/depth limits;
- MIME and magic-byte detection;
- secret/sensitive-file pre-scan;
- symlink and path-traversal defense.

### `qbm-snapshot`

- deterministic traversal;
- streaming hashes;
- ignore/include policy;
- large-artifact references;
- inventory canonicalization;
- snapshot commit and verification;
- unchanged-file reuse.

### `qbm-scanner-sdk` and `qbm-scanners`

Scanner trait:

```rust
pub trait Scanner: Send + Sync {
    fn descriptor(&self) -> ScannerDescriptor;
    fn accepts(&self, artifact: &InventoryArtifact) -> bool;
    fn scan(&self, ctx: ScanContext<'_>) -> Result<RawFactBatch, ScanError>;
}
```

Scanner implementation strategy:

| Source | Rust strategy |
|---|---|
| YAML/JSON/TOML | Serde-compatible parsers with span-preserving frontends |
| CSV/TSV | Streaming reader; schema inference remains explicitly provisional |
| Rust | Cargo metadata plus syntax-tree parsing; optional rustdoc JSON bridge |
| Python/JS/TS/etc. | Tree-sitter parsing; no imports or code execution |
| OpenAPI/JSON Schema | Typed document parser and reference resolver |
| GraphQL/protobuf | Schema parser and dependency graph |
| SQLite/PostgreSQL | Read-only schema introspection through connector |
| ONNX | Rust ONNX/protobuf metadata and operator graph inspection |
| PDF/text | Rust text extraction when possible; OCR as isolated optional tool |
| Git | Rust Git library or non-interactive Git subprocess behind adapter |

Every scanner supports cancellation, size limits, structured diagnostics, and source spans.

### `qbm-connector-sdk` and `qbm-connectors`

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    fn descriptor(&self) -> ConnectorDescriptor;
    async fn discover(&self, req: DiscoverRequest) -> Result<Discovery, ConnectorError>;
    async fn acquire(&self, req: AcquireRequest, sink: &dyn QuarantineSink)
        -> Result<ConnectorReceipt, ConnectorError>;
    async fn verify(&self, receipt: &ConnectorReceipt)
        -> Result<Verification, ConnectorError>;
}
```

Network clients use explicit host allowlists, rate limits, timeouts, bounded responses, TLS validation, secret handles, and redacted tracing. A connector cannot approve its own output.

### `qbm-adapter-sdk`

Contains:

- `ProjectAdapter` trait;
- detection evidence types;
- mapping DSL and compiled mapping representation;
- semantic graph builder;
- policy and runner declarations;
- projection builder contract;
- conformance harness;
- WASM/process plugin protocol.

Adapters see an immutable snapshot and scanner facts through read-only interfaces.

### `qbm-policy`

Policy execution should support two forms:

1. compiled Rust policies for complex, performance-critical checks;
2. a constrained declarative policy format for relationships, required attributes, severity, and remediation templates.

The declarative language must be non-Turing-complete in the first version. It may query graph patterns but cannot perform filesystem or network actions.

### `qbm-audit`

Provides reusable analyses:

- reference integrity;
- duplicate/equivalent entity detection;
- reachability and inert-node detection;
- cycle and topological checks;
- domination and shadow candidates;
- provenance completeness;
- schema compatibility;
- test-to-component coverage;
- sensitive-data leakage into exportable artifacts;
- report completeness.

### `qbm-diff`

- node/edge/hyperedge diff;
- rename candidate detection without silently treating it as confirmed;
- semantic attribute diff;
- behavior output diff;
- source, API, schema, model, and dataset diff;
- change classification and impact traversal.

### `qbm-runner`

Coordinates external execution through a hardened sandbox abstraction. The first implementation may use operating-system process isolation and container tools where available, but the domain contract must not depend on one sandbox technology.

Runner output:

```rust
struct BehaviorRunRecord {
    command_contract_hash: Digest,
    fixture_set_hash: Digest,
    environment_hash: Digest,
    exit: ExitRecord,
    stdout_artifact: ArtifactId,
    stderr_artifact: ArtifactId,
    outputs: Vec<ArtifactId>,
    metrics: Vec<MetricValue>,
    resource_usage: ResourceUsage,
}
```

### `qbm-objective`

- validates explicit objective contracts;
- resolves semantic node selectors;
- checks protected invariants;
- verifies backend representability;
- produces backend-neutral optimization instances;
- records the exact projection-to-instance transformation.

### `qbm-optimizer`

Rust-native initial backends:

- exhaustive enumeration for small instances;
- greedy and lazy-greedy coverage;
- simulated annealing;
- tabu search;
- branch-and-bound for selected problem families;
- ILP/MILP integration through a solver abstraction.

The result contract distinguishes:

```text
CERTIFIED_OPTIMAL
CERTIFIED_BOUND
HEURISTIC
INFEASIBLE
UNSUPPORTED
FAILED
```

### `qbm-qubo`

Port the validated mathematical semantics from the Python reference:

- maximum-coverage and minimum-input modes;
- exact inequality/slack handling;
- supported QUBO encodings;
- QUBO-to-Ising conversion;
- bit ordering and decode maps;
- penalty-bound validation;
- energy/native-objective parity checks;
- resource estimates before circuit construction.

Rust and Python implementations must be tested on identical serialized fixtures and compare exact coefficients/results within declared numeric tolerances.

### `qbm-quantum-protocol`

Provider-neutral types:

- prepared problem;
- Hamiltonian/circuit identity;
- logical variable map;
- provider/backend request;
- transpilation metadata;
- shot/sample output;
- provider job receipt;
- decoded and natively rescored solution;
- error mitigation and calibration metadata.

The Rust core prepares, approves, and validates the payload. A provider bridge only performs provider-specific conversion/submission and returns signed/hashed artifacts.

### `qbm-report`

- canonical report model;
- JSON and JSON Schema;
- CSV tables;
- Markdown;
- static HTML with embedded local assets;
- optional PDF through a separate renderer;
- stable finding links and evidence anchors;
- report manifest and checksums.

### `qbm-orchestrator`

- stage DAG;
- durable transitions;
- capability enforcement;
- approval waits;
- cancellation and retries;
- cache lookup;
- event emission;
- worker leases;
- stale-job recovery;
- run branching after `REQUEST_CHANGES`.

### `qbm-app`

Application use cases shared by CLI/API/UI:

```text
ImportProject
StartRun
ApproveStage
RejectStage
BuildSnapshot
ResolveAdapter
RunAudit
CompareVersions
RunOptimization
PrepareQuantumJob
PublishReport
```

## 4. Browser implementation in Rust

### Backend

- asynchronous Rust HTTP server;
- typed request/response models generated from or checked against the same schemas as domain artifacts;
- streamed upload and download;
- server-sent events for progress;
- bounded worker concurrency;
- graceful shutdown and job recovery;
- loopback binding by default.

### Frontend

Use Rust compiled to WebAssembly for:

- project/run state;
- approval views;
- finding filtering;
- diff tables;
- optimization forms;
- graph-query controls;
- API client types.

Server-side pagination and neighborhood queries prevent a huge project graph from being copied into browser memory.

Browser APIs may require a small JavaScript glue layer for folder selection, download, or platform compatibility. This is an integration shim, not a second application core.

## 5. Python bridge protocol

Use an external process rather than embedding Python in the Rust server initially.

```text
Rust orchestrator
  -> creates approved request.json
  -> starts bridge with restricted working directory
  -> sends protocol handshake over stdin
  -> bridge validates schema/version
  -> bridge writes progress/result messages to stdout
  -> binary/large output goes to approved artifact paths
  -> Rust verifies hashes and native objective
  -> Rust commits RunRecord
```

Protocol envelopes:

```json
{
  "protocol": "qbm.bridge/v1",
  "request_id": "...",
  "kind": "quantum.execute",
  "payload": {}
}
```

Standard output is protocol-only. Human logs go to standard error and are captured as an artifact. Unknown or out-of-order messages fail the bridge job.

## 6. Existing Python migration

### Stage A — Freeze parity fixtures

- retain current benchmark files and Python tests;
- serialize representative native objectives, QUBOs, Ising coefficients, decode maps, and solver results;
- record accepted numeric tolerances.

### Stage B — Rust readers and report integration

- Rust reads existing benchmark instance files;
- current Python experiments can be invoked through the bridge;
- results enter the new `RunRecord` format.

### Stage C — Port core mathematics

- native scoring;
- QUBO builders;
- Ising conversion;
- exact and heuristic classical solvers;
- local state-vector simulator if still justified.

Each ported function must pass cross-language parity before becoming default.

### Stage D — Python becomes optional

Python remains required only for Qiskit/IBM and selected ML integrations. Local structural audits, reports, graph analysis, and classical optimization run as Rust binaries.

## 7. Concurrency model

- Tokio for I/O-bound connectors, server work, and orchestration;
- bounded blocking pools for parsing and hashing;
- Rayon or equivalent bounded data parallelism for CPU-heavy independent analyses;
- cancellation tokens propagated to scanners and workers;
- per-project and global concurrency limits;
- memory budgets for graph and optimization jobs;
- streaming interfaces for bulk files.

Never spawn one task per file without bounds.

## 8. Database and artifact layout

Initial local directory:

```text
~/.qbenchmed/                       # configurable; never hard-coded in logic
├── qbm.sqlite
├── artifacts/
│   └── sha256/ab/cd/<full-hash>
├── quarantine/<upload-id>/
├── runs/<run-id>/workspace/
├── plugins/
└── logs/
```

The implementation must resolve this through an application-specific configuration value and must not repurpose generic system environment variables.

Metadata DB tables conceptually include:

```text
projects
snapshot_records
snapshot_pointers
artifact_records
runs
stage_runs
events
approval_requests
approval_decisions
findings
plugin_records
connector_receipts
```

Large graph payloads may be content-addressed artifacts with indexed summaries in SQLite. Do not force every node and edge into a single relational table before performance measurements justify it.

## 9. Error model

Use typed errors at crate boundaries. Convert them to:

- stable machine error code;
- safe user message;
- internal diagnostic chain;
- retryability;
- affected stage and artifact;
- optional finding.

Secrets and full external response bodies are redacted before tracing. Panics are treated as defects, caught at worker/process boundaries where possible, and never interpreted as ordinary validation failure.

## 10. Observability

Rust structured tracing records:

- run/stage/job IDs;
- artifact hashes rather than sensitive paths where possible;
- durations and queue times;
- bytes and records processed;
- cache hits;
- findings counts;
- external calls with credentials redacted;
- cancellation and retry reasons.

The report contains a reproducibility environment record, not unrestricted application logs.

## 11. Rust-first definition of done

The platform qualifies as Rust-first when:

- one Rust binary can run intake through report generation;
- CLI, API, orchestration, snapshots, graphs, approvals, diffs, reports, and generic audits contain no Python dependency;
- PATHEX and ONCOVA static adapters are Rust-native;
- classical baselines and QUBO construction are Rust-native with parity tests;
- Python processes are optional, isolated capabilities clearly shown in a run;
- a run without quantum or Python-only ML inspection starts no Python process.
