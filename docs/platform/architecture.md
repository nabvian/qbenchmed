# System Architecture

## 1. Architectural goals

The platform must be:

- universal across rule engines, data systems, software services, and ML projects;
- local-first and usable without an external cloud;
- deterministic for a fixed source snapshot, adapter, policy, engine, and objective;
- read-only toward source projects;
- resumable after a process restart;
- approval-gated and fully traceable;
- horizontally extensible through adapters, policies, scanners, runners, and analysis backends;
- useful without quantum computing;
- capable of isolating Python-only or provider-specific bridges from the Rust core.

## 2. Logical component model

```text
Presentation
  qbm-cli | qbm-web-ui | HTTP/SSE API
                         |
Application
  project service | run orchestrator | approval service | report service
                         |
Domain
  snapshots | graphs | adapters | policies | findings | objectives | run records
                         |
Engines
  scanners | validators | diff | behavior runners | optimizers | QUBO | reports
                         |
Infrastructure
  artifact store | metadata DB | sandbox | secret store | connectors | event log
```

Dependencies point inward. Domain types do not depend on Axum, SQLite, Qiskit, UI code, or a specific adapter.

## 3. Component responsibilities

### 3.1 Intake service

Accepts:

- a filesystem path registered by the local CLI;
- a browser directory upload;
- ZIP or TAR archives;
- an existing Git worktree;
- a connector manifest for bulk, API, database, or model sources.

It creates a quarantined intake record. It rejects unsafe archive paths, uncontrolled symlinks, excessive nesting, size-policy violations, and forbidden file classes. It does not execute the contents.

### 3.2 Inventory service

Creates a deterministic file and artifact inventory:

- normalized relative path;
- file type and media type;
- byte size;
- cryptographic hash;
- permissions and symlink status;
- probable sensitivity;
- scanner eligibility;
- exclusion reason;
- Git identity when available.

Files are processed in a canonical order so an unchanged project produces the same inventory hash.

### 3.3 Snapshot service

Freezes an approved intake as an immutable `SourceSnapshot`. Storage is content-addressed. Large artifacts may remain external when their manifest and hash are sufficient. A snapshot cannot be edited; a correction creates a new snapshot.

Snapshot identity is calculated from canonical metadata, artifact hashes, inclusion/exclusion rules, and connector receipts. Timestamps are recorded but excluded from the content identity unless they are part of the source version.

### 3.4 Scanner registry

Selects format-specific static scanners. Scanners emit raw facts and provenance, not domain conclusions. For example:

```text
Rust scanner: function A calls function B
YAML scanner: object X contains reference Y
ONNX scanner: tensor T has dimensions D and feeds operator O
OpenAPI scanner: operation P accepts schema S and returns schema R
```

### 3.5 Adapter registry

Ranks candidate adapters using explicit manifests, signatures, project markers, and scanner evidence. Detection returns confidence plus reasons. Ambiguous detection pauses for approval instead of silently choosing.

An adapter converts raw facts to generic semantic types and declares supported benchmark projections. Project-specific policy packs run alongside the adapter.

### 3.6 Graph service

Stores immutable graph layers:

- inventory graph;
- code/dependency graph;
- rule graph;
- data-lineage graph;
- model computation/signature graph;
- evidence/knowledge graph;
- approved semantic graph;
- benchmark projection graphs.

Graph indexes support reachability, strongly connected components, topological ordering, dominance, unused-node analysis, lineage, impact analysis, and graph diff.

### 3.7 Policy engine

Evaluates generic and adapter-provided policies. A policy receives a versioned graph and produces findings. Policies cannot mutate the graph. Findings include evidence and deterministic identifiers.

Policy categories include:

- structure;
- provenance and licence;
- privacy and secret exposure;
- clinical/scientific governance;
- model validation;
- reproducibility;
- execution safety;
- optimization eligibility;
- external export eligibility.

### 3.8 Behavioral runner

Runs only approved commands in an isolated workspace using declared fixtures. It captures stdout, stderr, exit status, environment identity, timings, resource usage, and generated artifact hashes.

The runner has no network by default. A project-supplied shell script is not trusted merely because it exists. Its command, inputs, expected outputs, filesystem permissions, environment variables, and network policy must be declared in the runner contract and approved.

### 3.9 Diff engine

Compares immutable graph and behavior snapshots. It separates identity changes, structural changes, semantic changes, behavior changes, metadata changes, and provenance-only changes. It can compare any two versions; it never assumes the newest is approved.

### 3.10 Analysis engine

Runs graph analyses and explicit objective contracts. Each backend advertises supported problem types and constraints. The engine refuses a job when required semantics cannot be represented exactly.

Backend families:

- graph algorithms;
- exact enumeration for small instances;
- exact ILP/MILP;
- greedy and approximation methods;
- simulated annealing and tabu;
- QUBO construction and classical QUBO solvers;
- local QAOA simulator;
- external quantum provider bridge.

### 3.11 Report service

Combines manifests, approvals, graphs, findings, diffs, behavior results, optimization results, limitations, and environment data into deterministic JSON, CSV, Markdown, and HTML artifacts.

### 3.12 Approval service

Records append-only decisions bound to exact stage-output hashes. If a stage is rerun and its output changes, the old approval is stale and cannot authorize the new output.

Supported decisions:

- `APPROVE` — continue;
- `REJECT` — terminate this run branch;
- `REQUEST_CHANGES` — return to a named earlier stage with a reason;
- `WAIVE` — allowed only when policy explicitly permits a documented exception.

## 4. Orchestrated stage machine

```text
CREATED
  -> INTAKE
  -> INVENTORY
  -> SENSITIVITY_CLASSIFICATION
  -> SOURCE_SNAPSHOT
  -> ADAPTER_DETECTION
  -> RAW_GRAPH
  -> SEMANTIC_MAPPING
  -> STRUCTURAL_AUDIT
  -> BEHAVIORAL_AUDIT
  -> PROVENANCE_POLICY_AUDIT
  -> VERSION_DIFF
  -> PROJECTION_BUILD
  -> OPTIMIZATION
  -> OPTIONAL_QUANTUM
  -> REPORT
  -> PUBLISH
  -> COMPLETE
```

Every stage has one of these states:

```text
PENDING | RUNNING | WAITING_APPROVAL | APPROVED | REJECTED |
FAILED | BLOCKED | CACHED | SKIPPED | COMPLETE
```

The pipeline definition declares for each stage:

- prerequisites;
- input artifact types;
- output artifact types;
- retry policy;
- cache key;
- required capabilities;
- approval policy;
- failure and rollback behavior;
- whether a stage may use network, execute code, or export data.

Onboarding mode requires approval at every material stage. Continuous mode may auto-approve mechanically identical low-risk stages according to a signed policy, but semantic, safety, execution, external export, and publication gates remain manual unless governance explicitly changes them.

## 5. Event-driven execution

Every state transition produces an append-only event:

```text
RunCreated
StageScheduled
StageStarted
ArtifactProduced
FindingRaised
StageCompleted
ApprovalRequested
ApprovalRecorded
StageRejected
RunCompleted
```

The database stores the current projection of state; the event log is the audit trail. A restart reconstructs or verifies run state before work resumes.

Workers claim idempotent stage jobs. A job is keyed by:

```text
hash(stage kind + input hashes + engine version + configuration + policy version)
```

An identical safe job may reuse cached output. Behavioral jobs with nondeterministic components must record seeds, repetitions, and non-cacheable environment fields.

## 6. Local-first deployment

The first supported deployment is one local process distribution:

```text
qbm serve
  -> starts local HTTP service
  -> starts worker pool
  -> opens/serves browser UI
  -> uses local SQLite metadata DB
  -> uses local content-addressed artifact directory
```

CLI-only operation uses the same application services without starting the browser UI.

Later deployment may separate:

- API server;
- worker pool;
- PostgreSQL metadata store;
- object storage;
- identity provider;
- remote sandbox workers.

The domain model and stage protocol must remain the same in local and multi-user modes.

## 7. Data and control boundaries

```text
Source project             read-only
Quarantine                 writeable, untrusted
Snapshot store             immutable after commit
Run workspace              ephemeral and isolated
Metadata/event database    append/update through services only
Report export              explicit destination and approval
External API/quantum       denied unless stage capability is approved
```

The platform must distinguish data acquisition from decision-time use. For ONCOVA-style systems, bulk and API connectors refresh the local mirror; audits and behavioral fixtures reference a frozen snapshot.

## 8. Failure behavior

- Parser failure creates a scoped finding and does not silently omit the file.
- Hash mismatch stops snapshot promotion.
- Ambiguous adapter detection pauses for a decision.
- Missing semantic mapping prevents affected projections, while unaffected structural audits may continue.
- Failed behavior runners do not invalidate static findings; their audit section is marked incomplete.
- An unavailable API leaves the last approved snapshot intact.
- A rejected stage stops dependent stages but preserves all prior artifacts and evidence.
- A report states every skipped, incomplete, or unverifiable check.

## 9. Extension boundaries

Use these stable extension points:

- `Scanner` — bytes/files to raw facts;
- `Connector` — external source to staged artifacts and receipts;
- `Adapter` — raw facts to semantic graph and projections;
- `Policy` — graph/run data to findings;
- `Runner` — approved fixture contract to behavior records;
- `Analyzer` — graph/projection to metrics;
- `Optimizer` — objective contract to candidate solutions;
- `QuantumProvider` — approved abstract job to provider result;
- `Reporter` — canonical run model to output format.

Third-party extensions should use a versioned process or WASM boundary. Rust dynamic-library ABI is not the public plugin contract.

