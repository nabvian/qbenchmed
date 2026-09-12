# Security, Testing, Migration, and Build Roadmap

## 1. Trust model

Assume every imported project, archive, document, model, manifest, generated report fragment, adapter plugin, and external API response is untrusted until validated.

Primary assets to protect:

- user source code and intellectual property;
- patient, research, financial, and other sensitive data;
- API credentials and provider tokens;
- approved graph and snapshot history;
- audit integrity and approval records;
- local workstation and filesystem;
- correctness of findings and optimization results;
- external service budgets and quotas.

Primary adversarial or accidental risks:

- path traversal, symlink escape, archive bomb, or excessive file count;
- parser denial of service or malformed-file crash;
- uploaded code executing during inspection;
- secret or patient-data leakage into reports or quantum payloads;
- malicious plugin or behavior runner;
- API response/schema drift;
- stale approval applied to changed output;
- graph/parser omission mistaken for a successful audit;
- hash/provenance substitution;
- objective specification that optimizes the wrong proxy;
- resource exhaustion from large graphs, models, or QUBOs;
- cloud submission of proprietary benchmark structure.

## 2. Capability model

Stages request explicit capabilities:

```text
READ_APPROVED_SNAPSHOT
WRITE_RUN_WORKSPACE
EXECUTE_DECLARED_COMMAND
READ_MODEL_ARTIFACT
ACCESS_LOCAL_DATABASE_READONLY
NETWORK_TO_ALLOWLIST
READ_SECRET_HANDLE
EXPORT_REPORT
EXPORT_QUANTUM_PAYLOAD
PUBLISH_SNAPSHOT
```

A capability is granted by global policy, project policy, and any required approval. Possessing one capability does not imply another. For example, an API connector may use one allowlisted host but cannot execute project code or publish an approved snapshot.

## 3. Intake controls

- stream uploads to quarantine;
- normalize and validate every relative path;
- reject absolute paths, parent traversal, device files, sockets, and unexpected hard links;
- do not follow symlinks by default;
- enforce compressed and expanded size limits;
- enforce file count, path length, nesting depth, and per-file limits;
- identify type using content as well as extension;
- scan for secrets and sensitive path patterns before report/export eligibility;
- store quarantine under a run-specific, non-shared directory;
- never render arbitrary HTML/SVG with active content in the trusted application origin;
- remove executable permission in quarantine where supported;
- use read-only snapshot views for scanners.

## 4. Static scanning controls

- parsers receive bounded input;
- recursion and allocation limits are configured;
- parsing does not resolve includes outside the snapshot;
- YAML aliases/references have expansion limits;
- document hyperlinks are data, not actions;
- imports are parsed, not executed;
- build scripts, macros, notebooks, and package hooks do not run;
- scanner errors become explicit incomplete-check records;
- third-party scanners run out of process or in WASM when possible;
- fuzz parser and canonicalization boundaries.

## 5. Execution sandbox

Behavioral auditing is a distinct approved stage.

Minimum runner controls:

- ephemeral workspace populated only with approved artifacts and fixtures;
- source mounted read-only where possible;
- dedicated unprivileged identity;
- network denied by default;
- explicit environment allowlist;
- secrets referenced by handles and redacted from output;
- wall-clock, CPU, memory, process-count, file-size, and output limits;
- no access to the Q-BenchMed metadata DB or artifact store except through a narrow output sink;
- captured command contract, executable hash, environment, and exit result;
- kill and cleanup on cancellation or limit violation.

Containers alone are not a complete security boundary. Use operating-system sandbox features and document platform-specific limitations.

## 6. Connectors and API security

- TLS verification is mandatory;
- endpoint allowlists are per connector;
- redirects are restricted and revalidated;
- credentials live in the OS credential store or approved secret provider;
- credentials never enter manifests, logs, graphs, reports, or adapter input;
- responses have byte and record limits;
- pagination and cursors are recorded;
- retry is idempotent;
- rate limits and provider terms are respected;
- raw responses are mirrored and hashed before parsing;
- licence and privacy classification precede promotion;
- data deletion/tombstones are represented explicitly;
- live API data cannot bypass snapshot approval.

## 7. Privacy and sensitive data

Sensitivity levels:

```text
PUBLIC
INTERNAL
RESTRICTED
PROTECTED
FORBIDDEN
```

Each artifact, graph node, projection, report section, and export inherits or raises sensitivity; transformations cannot silently lower it.

Rules:

- patient-identifiable information is `FORBIDDEN` in benchmark and quantum artifacts;
- behavior fixtures use synthetic or properly governed de-identified data;
- large clinical/research datasets remain in controlled storage;
- reports use relative paths and safe excerpts, not full sensitive content;
- source excerpts have length and sensitivity limits;
- external export computes a payload manifest and requires policy validation;
- deletion/retention settings apply to quarantine and ephemeral workspaces without deleting immutable governed evidence unexpectedly.

## 8. Approval integrity

- approval binds run ID, stage ID, stage output hash, and policy version;
- optimistic concurrency prevents decisions against stale output;
- decisions are append-only;
- optional local signatures support higher-assurance deployments;
- event hashes form a tamper-evident chain;
- changing an approved artifact invalidates dependent approvals;
- roles are separated for regulated profiles;
- no adapter, connector, scanner, or optimizer can approve its own output;
- emergency override, if ever implemented, is explicit, separately authorized, and impossible for non-waivable privacy/integrity failures.

## 9. Report security

- escape all project-controlled text;
- sanitize Markdown/HTML;
- render unknown SVG/HTML as download-only;
- never embed secrets or raw protected samples;
- content security policy for the local report viewer;
- no remote scripts, fonts, tracking, or analytics;
- checksums for every report artifact;
- report manifest lists removed/redacted evidence;
- exported report carries its sensitivity and sharing warning.

## 10. Supply-chain and plugin security

- locked Rust and Python dependencies;
- software-bill-of-materials generation;
- dependency licence and vulnerability audit in CI;
- signed official release artifacts;
- plugin ID, version, publisher, hash, schema compatibility, and capability declaration;
- third-party plugin install is explicit;
- WASM/process isolation for non-built-in plugins;
- adapter output is schema-validated regardless of trust level;
- plugins cannot declare themselves approved.

## 11. Test strategy

### 11.1 Unit tests

- canonical serialization and hash stability;
- ID and state-transition invariants;
- path normalization;
- graph algorithms;
- mapping and condition-group construction;
- policy queries;
- diff classification;
- objective validation;
- native scoring, QUBO, Ising, and decode logic;
- report escaping and schema validation.

### 11.2 Property tests

- canonical serialization is idempotent;
- graph round-trip preserves identity;
- snapshot order does not affect hash;
- unchanged input produces unchanged snapshot;
- an edge never references a missing node after validation;
- approvals fail for changed stage hashes;
- native objective and exact QUBO ordering agree on enumerable instances;
- decoded solutions are rescored consistently;
- diff(A, A) is empty;
- diff inversion swaps additions and removals.

### 11.3 Fuzz tests

- archive paths and headers;
- YAML/JSON/TOML parsers and reference resolution;
- manifest/schema parsers;
- bridge protocol;
- graph import;
- report rendering;
- multipart upload;
- API fixture parsing.

### 11.4 Golden fixtures

Keep small, synthetic, non-sensitive repositories that deliberately contain:

- duplicate IDs;
- broken references;
- AND/OR paths;
- cycles and stage-order violations;
- inert inputs and unreachable targets;
- model-signature drift;
- API-schema drift;
- provenance gaps;
- secret-like strings;
- version renames and removals.

Expected inventories, graphs, findings, diffs, and reports are versioned.

### 11.5 Adapter conformance

Every adapter must prove:

- deterministic detection and export;
- no source mutation;
- complete provenance on semantic elements;
- unknown/new source elements are surfaced;
- conjunction and constraint semantics are preserved;
- policy and projection catalog is versioned;
- unsupported projections fail clearly;
- sensitive data is not exported;
- a snapshot change invalidates cached output correctly.

### 11.6 Cross-language parity

For the current Python Q-BenchMed and Rust ports:

- identical instance parsing;
- identical native objective;
- coefficient-level QUBO comparison;
- Ising energy parity;
- bit-order and decode parity;
- exact optimum parity;
- declared tolerance for floating-point heuristic traces;
- report labels distinguishing deterministic from stochastic differences.

### 11.7 End-to-end tests

#### Generic project

Upload a synthetic mixed YAML/Rust/API project, approve each stage, receive the expected report, reject one stage, resume a corrected branch, and verify immutable history.

#### PATHEX

Export rule semantics, preserve condition groups, build coverage and behavior projections, compare two versions, and verify no patient fixture enters the report.

#### ONCOVA

Import GUARD and CORPUS, verify research-only status, run declared local gates, detect corpus scope/veto gaps, compare snapshots, and prove research graph separation.

#### Large bulk

Process a generated large manifest through streaming code with bounded memory and no raw duplication.

#### API delta

Replay paginated API fixtures, inject schema drift and partial failure, produce a quarantined delta, and prove the previous approved snapshot is unchanged.

#### Quantum bridge

Use an 8–12 input synthetic instance, compare exact Rust result, local reference simulator, Python/Qiskit bridge, and decoded/native-rescored output.

## 12. Performance requirements

Set measured targets after prototype profiling, but preserve these architectural constraints from the beginning:

- streaming intake and hashing;
- bounded parser and worker concurrency;
- incremental snapshots and graph updates;
- neighborhood/paginated graph APIs;
- cancellation at stage and worker level;
- explicit memory budgets;
- optimization size estimation before construction;
- QUBO/qubit resource estimate before simulation or hardware preparation;
- no browser transfer of an entire unbounded graph.

Performance results must be reported by fixture size and environment, not as unsupported universal claims.

## 13. Migration strategy for the current repository

The present project is a Python reference for input-selection/QUBO experiments. Preserve it as the mathematical oracle while adding the platform.

### Migration rule

```text
Do not rewrite first.
Define contracts -> wrap existing behavior -> freeze parity -> port component by component.
```

### Compatibility requirements

- existing benchmark files remain loadable;
- existing experiment commands continue to work during early phases;
- current report results are reproducible or differences are documented;
- Rust output can be compared against current Python tests;
- the new platform does not claim that current synthetic Heme data is a live PATHEX export.

## 14. Build roadmap

### Milestone 0 — Architecture freeze

Deliver:

- reviewed platform documents;
- product/non-goal agreement;
- threat model;
- schema naming and version policy;
- approval stage list;
- two onboarding fixtures selected.

Exit criteria: no unresolved contradiction about domain blindness, source mutation, clinical authority, or live API use.

### Milestone 1 — Rust foundation

Build:

- workspace and CI;
- `qbm-domain`, `qbm-canonical`, and `qbm-store`;
- project/run/artifact IDs;
- SQLite migrations;
- content-addressed local artifact store;
- event and approval records;
- basic CLI.

Exit criteria: create a project and run, store an artifact, record/reload stage events, and reject stale approvals.

### Milestone 2 — Safe intake and snapshots

Build:

- local folder and archive intake;
- quarantine;
- file inventory;
- include/exclude policy;
- hashing and snapshot commit;
- secret/sensitivity pre-scan;
- upload/path CLI and API.

Exit criteria: unchanged folders create the same content identity; unsafe archives and symlinks are rejected; source is not modified.

### Milestone 3 — Raw graph and generic audit

Build:

- YAML/JSON/TOML, Rust, Python, OpenAPI, documentation, and test scanners;
- raw graph schema/store;
- duplicate/reference/cycle/reachability/provenance policies;
- finding schema;
- JSON/Markdown report.

Exit criteria: mixed synthetic fixture produces complete, evidence-linked expected findings.

### Milestone 4 — Adapter SDK and semantic graph

Build:

- adapter traits and mapping DSL;
- detection registry;
- semantic graph builder;
- policy-pack loading;
- projection catalog;
- conformance harness;
- `generic-manifest` adapter.

Exit criteria: a new configuration-only adapter can be added without changing audit-core code.

### Milestone 5 — PATHEX adapter

Build:

- rules/input/outcome extraction;
- selectable/context/derived classification;
- conjunction-preserving rule paths;
- provenance and unit policies;
- coverage, minimum-panel, reachability, and version projections;
- controlled behavior-runner contract.

Exit criteria: approved PATHEX fixture exports deterministic profiles and reports graph coverage separately from behavior preservation.

### Milestone 6 — ONCOVA adapter and connectors

Build:

- GUARD, CORPUS, retrieval, execution graph, and research graph mappings;
- local bulk connector;
- API-delta receipt and replay framework;
- source/licence/review-state policies;
- existing ONCOVA gate runner integration;
- frozen corpus-snapshot diff.

Exit criteria: ONCOVA folder completes onboarding/audit, API fixtures cannot bypass the local snapshot, and clinical/research boundaries remain intact.

### Milestone 7 — Approval UI and full reports

Build:

- local Rust HTTP server;
- Rust/WASM project import UI;
- run timeline and SSE progress;
- semantic mapping review;
- approval/reject/request-changes actions;
- findings, diff, graph, and report views;
- static HTML report bundle.

Exit criteria: the ONCOVA and PATHEX acceptance workflows can be completed entirely in the browser or entirely through the CLI.

### Milestone 8 — Behavioral and ML support

Build:

- sandbox abstraction;
- golden-fixture execution;
- API record/replay;
- model metadata and ONNX scanner;
- metrics and regression engine;
- model-lineage projection.

Exit criteria: no uploaded code executes without an approved runner capability; model and API regressions are evidence-linked.

### Milestone 9 — Rust optimization backends

Build:

- objective validator;
- graph-native coverage/reachability analyses;
- exact small-instance solver;
- ILP abstraction;
- greedy, annealing, and tabu;
- Rust QUBO/Ising port;
- Python parity suite;
- optimization UI/report.

Exit criteria: current flagship and synthetic fixtures reproduce accepted native results; every solution is feasibility checked and natively rescored.

### Milestone 10 — Optional quantum bridge

Build:

- provider-neutral quantum protocol;
- Qiskit bridge;
- local Qiskit parity;
- IBM provider adapter;
- payload approval and external-export manifest;
- hardware result decoding and reporting.

Exit criteria: a small synthetic instance runs end to end with exact/local/provider comparison, no sensitive data, and a fully recorded payload.

### Milestone 11 — Hardening and release

Build:

- fuzz/property coverage targets;
- dependency/SBOM/signing pipeline;
- backup/restore and schema migration tests;
- platform-specific sandbox validation;
- performance and large-bulk fixtures;
- plugin signing/capability UI;
- operator and adapter-author documentation.

Exit criteria: reproducible signed local release, recovery-tested run history, security test report, and two unrelated real adapters with no core fork.

### Milestone 12 — Open-source community release

Build:

- zero-configuration `qbm audit .` experience;
- guided project-profile and configuration-adapter wizard;
- plugin pack/install/test/publish commands;
- decentralized community plugin index;
- SARIF, JUnit, JSON, Markdown, and HTML CI outputs;
- signed macOS, Linux, Windows, and container release artifacts;
- licence, governance, contribution, conduct, security, support, release, and roadmap documents;
- quick starts, example projects, adapter starter kits, and CI recipes;
- accessibility and offline/privacy release checks.

Exit criteria: a new user installs one artifact, audits an unknown project without a manifest or Python, generates a private adapter without rebuilding the core, and can reproduce the same workflow from the browser or CLI.

## 15. Priority rule

The dependency order is:

```text
Identity and snapshots
  -> graph truth
  -> adapter truth
  -> audits and approvals
  -> behavior
  -> optimization
  -> quantum
```

Do not begin with browser polish or IBM integration before snapshot identity, adapter semantics, and native result validation are stable.

## 16. Definition of platform completion

The universal platform is complete for its first release when:

1. A user can import a folder/archive/path from CLI or browser.
2. No source file is modified.
3. Intake, snapshot, adapter, semantic, audit, diff, optimization, and report stages are resumable.
4. Every configured gate supports approve, reject, and request changes.
5. Approvals are bound to output hashes.
6. PATHEX and ONCOVA run through the same core without core forks.
7. Large bulk and API-delta inputs become immutable local receipts/snapshots.
8. Reports list evidence, limitations, passed/failed/skipped checks, and history.
9. Classical analysis is useful with quantum disabled.
10. Quantum export is optional, abstract, approved, and natively rescored.
11. Rust owns the platform; Python is an isolated optional bridge.
12. Clinical/scientific claims remain under qualified human control.
13. An unknown project receives a useful generic audit without a custom adapter.
14. A user can generate and test a configuration adapter without writing Rust.
15. Public plugin, report, CLI, and API contracts are versioned and documented.
16. The default release operates offline with telemetry disabled.
17. Signed cross-platform artifacts, SBOM, governance, contribution, and security documentation are published.
