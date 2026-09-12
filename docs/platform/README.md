# Q-BenchMed Universal Audit Platform — Project Blueprint

Status: design specification, not an implementation claim  
Primary implementation language: Rust  
Existing Python Q-BenchMed role: validated reference and optional quantum bridge  
Deployment target: local-first CLI plus a minimal browser UI

## 1. Product definition

Q-BenchMed will become a universal, local-first project audit and benchmark platform. A user gives it a project folder, archive, Git worktree, local bulk-data manifest, or approved API source. The platform inventories the project, freezes an immutable snapshot, selects an adapter, builds project graphs, runs audits and optional optimizations, and produces a structured report.

The platform is not a replacement for PATHEX, ONCOVA, a finance engine, a neural engine, or any other production system. Those systems perform their domain work. Q-BenchMed independently inspects their structure, provenance, behavior, changes, and mathematical projections.

```text
PATHEX / ONCOVA / another project = operating system
Q-BenchMed                         = audit, test and optimization laboratory
```

The platform must never silently change a source project. It produces findings, comparisons, benchmark results, and proposed remediation. A person decides whether any source change is acceptable.

## 2. User experience

The simplest supported workflow is:

```text
Open local Q-BenchMed UI
    -> choose a folder or upload an archive
    -> Q-BenchMed inventories it
    -> review the stage summary
    -> APPROVE, REJECT, or REQUEST CHANGES
    -> Q-BenchMed continues automatically
    -> repeat at configured gates
    -> receive a complete audit package
```

The equivalent CLI workflow is:

```bash
qbm project import /path/to/project
qbm run start <project-id> --mode onboarding
qbm run approve <run-id> --stage inventory
qbm run approve <run-id> --stage semantic-mapping
qbm run approve <run-id> --stage audit
qbm report open <run-id>
```

“Automatic” means that scanning, parsing, hashing, graph construction, validation, diffing, analysis, optimization, and report generation require no manual file-by-file work. It does not mean that software invents clinical, scientific, financial, or safety meaning. The first onboarding of a new project or adapter requires approval of its semantic mapping and policy pack. Later versions can normally be audited with only approval decisions.

## 3. Foundational design philosophy

### 3.1 Snapshot first

Q-BenchMed audits immutable local snapshots, never a changing live source.

```text
API / bulk files / project folder
    -> stage locally
    -> validate
    -> hash
    -> freeze snapshot
    -> audit the snapshot
```

An API is acquisition plumbing. A bulk dataset is an acquisition artifact. Neither is the benchmark identity. The snapshot hash is the benchmark identity.

### 3.2 Three representations

Every onboarded project has three distinct representations:

1. **Raw Project Graph** — facts automatically discovered from files, code, schemas, models, tests, APIs, datasets, and documentation.
2. **Approved Semantic Graph** — human-approved meanings such as input, context, derived feature, constraint, safety gate, model output, evidence claim, and optimizable target.
3. **Benchmark Projection** — the smallest safe mathematical view required for a specific question.

The complete project graph must not be forced into the current input-to-outcome coverage matrix.

### 3.3 Domain-blind core, domain-aware edges

The core understands generic concepts: typed nodes, typed edges, hyperedges, constraints, evidence, provenance, versions, policies, tests, and findings. It does not contain oncology, hematology, finance, education, or cybersecurity rules.

Adapters and policy packs supply domain meaning. For example:

```text
ONCOVA adapter       contraindication -> safety constraint
PATHEX adapter       laboratory value -> selectable or observed input
Finance adapter      regulatory limit -> hard constraint
Cybersecurity adapter security control -> decision variable
Education adapter    learning objective -> coverage target
```

### 3.4 Classical first, quantum optional

File discovery, graph analysis, diffs, validations, report generation, and practical optimization are classical jobs. Exact ILP, graph algorithms, greedy methods, simulated annealing, and tabu are the default analysis backends.

QUBO, local QAOA, and quantum hardware are optional research backends. A quantum backend may consume only an explicitly approved, abstract benchmark projection. It may not receive patient data, secrets, private source text, or uncontrolled domain payloads.

### 3.5 Read-only by default

Project sources are mounted or opened read-only. Q-BenchMed writes only to its own content-addressed artifact store. Executing project code, calling external APIs, publishing a report, exporting a benchmark, or submitting a quantum job are separate capabilities requiring policy permission and, where configured, approval.

### 3.6 Evidence before conclusion

Every finding must point to evidence: a source path and span, graph node, test result, API contract, dataset manifest, model signature, or earlier snapshot. Unsupported prose is not an audit finding.

### 3.7 Immutable history

The platform keeps all snapshots and approval events. It exposes:

- `latest-discovered`: newest mechanically created snapshot;
- `latest-validated`: newest snapshot that passed technical validation;
- `latest-approved`: newest snapshot approved for governed use.

Normal audits default to `latest-approved`. Historical snapshots are never overwritten.

## 4. End-to-end architecture

```text
                         Q-BenchMed local trust boundary

 Folder / archive / Git / API / bulk / DB / model registry
                          |
                          v
                  [1] Intake + quarantine
                          |
                          v
                  [2] Inventory scanners
                          |
                          v
                 [3] Immutable snapshot
                          |
                          v
                 [4] Adapter detection
                          |
                          v
                  [5] Raw Project Graph
                          |
                    approval gate
                          |
                          v
               [6] Approved Semantic Graph
                          |
             +------------+-------------+
             |            |             |
             v            v             v
        structural    behavioral    provenance/
          audit          audit       policy audit
             |            |             |
             +------------+-------------+
                          |
                          v
                [7] Benchmark projections
                          |
             +------------+-------------+
             |                          |
             v                          v
      classical analysis       optional QUBO/quantum
             |                          |
             +------------+-------------+
                          |
                          v
                [8] Version comparison
                          |
                          v
                 [9] Audit report set
                          |
                    approval gate
                          |
                          v
                publish/export/archive
```

## 5. Audit levels

### Level 1 — Structural

Fully automatable after intake approval:

- duplicate and missing identifiers;
- broken references;
- cycles and invalid execution ordering;
- unreachable targets and inert inputs;
- malformed AND/OR logic;
- schema and model-signature mismatch;
- missing provenance, version, checksum, or licence;
- API contract and database-schema drift;
- unused files, rules, stages, tests, and models.

### Level 2 — Behavioral

Automatable when controlled runners or fixtures exist:

- old-versus-new output agreement;
- safety-output preservation;
- rule and retrieval regressions;
- performance, latency, and memory regressions;
- model accuracy/calibration changes on an approved evaluation set;
- reduced-input behavior preservation;
- API record/replay contract tests.

Project code is never executed merely because it was uploaded. Execution requires an approved runner profile and an isolated execution stage.

### Level 3 — Scientific, clinical, or regulated review

Q-BenchMed organizes the evidence but cannot supply the conclusion:

- whether a medical threshold is correct;
- whether a treatment direction is safe;
- whether a biological relationship is true;
- whether a model is clinically deployable;
- whether a regulatory or financial policy is valid;
- whether a proposed reduced input panel is acceptable.

These questions must remain assigned to qualified reviewers.

## 6. Universal source support

The platform supports source scanners and connectors for:

- YAML, JSON, TOML, CSV, XML, and schema files;
- Rust, Python, JavaScript/TypeScript, and other tree-sitter-supported code;
- OpenAPI, GraphQL, protobuf, and recorded service contracts;
- SQLite, PostgreSQL, and database schema snapshots;
- PDFs, HTML mirrors, and text corpora;
- ONNX metadata and computation graphs;
- PyTorch/TensorFlow model identity and signatures through controlled bridges where necessary;
- GNN/CNN graph, tensor, preprocessing, output, metric, and lineage metadata;
- Git history, manifests, lockfiles, tests, and documentation;
- local bulk datasets and scheduled API deltas.

Large or sensitive artifacts remain outside the graph. The graph stores their identity, version, checksum, lineage, schema, permissions, and approved derived metadata.

## 7. Adapter strategy

The initial adapter portfolio is:

1. `generic-manifest` — any project with `.qbench/project.yaml`;
2. `software-graph` — modules, dependencies, APIs, database schemas, tests;
3. `pathex` — rules, observations, derived values, contexts, outcomes, behavior fixtures;
4. `oncova` — GUARD, CORPUS, retrieval, execution pipeline, research knowledge graph;
5. `ml-model` — preprocessing, model signatures, lineage, evaluation suites;
6. `synthetic` — generated benchmark instances and current Q-BenchMed datasets.

New projects should prefer configuration and mapping packs over new code. A new adapter is needed only when the source semantics or extraction procedure cannot be expressed by an existing adapter.

## 8. What optimization means

Q-BenchMed does not optimize “a project” without a contract. Each optimization must declare:

- decision variables;
- targets or objective terms;
- hard and soft constraints;
- costs and weights;
- protected safety invariants;
- evaluation data and success threshold;
- allowed algorithms;
- whether quantum export is permitted.

Safe example:

```text
Select at most 20 assays, preserve every required safety pathway, and maximize
weighted evidence coverage on evaluation set EVAL-004.
```

Unsafe and rejected example:

```text
Optimize oncology.
```

Optimization results are proposals and measurements, not automatically applied changes.

## 9. Browser UI scope

The minimal browser UI provides:

- folder/archive import;
- project and snapshot list;
- adapter detection and confidence display;
- stage timeline with logs and artifacts;
- `APPROVE`, `REJECT`, and `REQUEST CHANGES` controls;
- findings table with severity, evidence, impact, and remediation;
- source/semantic/projection graph views;
- old-versus-new diff view;
- optimization configuration and results;
- report viewer and export;
- local settings, policy, and connector status.

The local Rust server embeds or serves the UI. The CLI and UI call the same application service; neither owns business logic.

## 10. Required deliverables

An implementation is not complete until it supplies:

1. Rust workspace and stable domain types;
2. safe intake, inventory, hashing, and immutable snapshot store;
3. raw, semantic, and projection graph schemas;
4. adapter SDK and conformance suite;
5. generic scanner and policy packs;
6. PATHEX and ONCOVA adapters;
7. structural, behavioral, provenance, and diff engines;
8. objective contract and classical analysis backends;
9. optional Python/IBM quantum bridge;
10. orchestrator with resumable approval gates;
11. CLI, HTTP API, and minimal browser UI;
12. deterministic structured report package;
13. security, sandbox, privacy, and licence controls;
14. end-to-end golden fixtures for at least two unrelated projects;
15. zero-configuration generic audit for unknown projects;
16. guided manifest and configuration-adapter generation;
17. versioned third-party plugin protocol and community index;
18. signed cross-platform releases, CI formats, governance, and contributor documentation.

## 11. Open-source universality

The public product must deliver useful results before a user writes an adapter.

```text
qbm audit .
  -> generic inventory and static audit
  -> automatic framework/adapter detection
  -> local structured report
  -> guided profile/adapter creation for deeper semantics
```

The default distribution is one local Rust application containing the CLI,
server, embedded browser UI, generic scanners, generic policies, report engine,
and sample projects. It requires no account, cloud service, Python runtime, or
manual manifest for the first audit. Telemetry and project upload are off by
default.

Community adapters use a stable configuration, WASM, or process-plugin
contract. They can be installed from a public index, Git release, private
registry, or local file. Registry access is optional and never required to run
the product. PATHEX and ONCOVA are built-in/example adapters, not assumptions in
the universal core.

The complete distribution, governance, packaging, plugin, CI, documentation,
and community design is in [Open-source product and ecosystem](open-source-product.md).

## 12. Non-goals

Q-BenchMed does not:

- diagnose, prescribe, stage, or choose treatment;
- certify scientific or clinical truth;
- train arbitrary models as part of an audit;
- upload protected data to a cloud service by default;
- execute untrusted project code during static scanning;
- automatically promote extracted evidence to an approved knowledge tier;
- modify source projects without a separate future remediation workflow;
- claim quantum advantage merely because QAOA or hardware was used.

## 13. Detailed specifications

- [System architecture](architecture.md)
- [Project graphs, snapshots, adapters, and connectors](data-model-and-adapters.md)
- [Workflow, approvals, CLI, API, and browser UI](interfaces-and-workflow.md)
- [Rust implementation map](rust-implementation.md)
- [Audit, optimization, diff, and report contracts](audit-and-reporting.md)
- [Security, testing, migration, and build roadmap](security-testing-roadmap.md)
- [Open-source product and ecosystem](open-source-product.md)
