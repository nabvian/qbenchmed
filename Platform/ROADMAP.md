# Build Roadmap

Q-BenchMed Platform is scoped as **A Modular Open-Source Quantum–Classical
Benchmarking Framework for Biomedical Input Optimization**.

Milestone status describes what is present in the source tree. It is not a
release-quality claim. A published release must also satisfy the verification
and hardening gates at the end of this document.

## Milestone 1 — Foundation

Status: implemented

- Rust workspace and domain contracts
- canonical serialization and hashing
- SQLite metadata and tamper-evident run events
- content-addressed artifact storage
- projects, runs, stages, exact-hash approvals, and CLI

Record: [docs/MILESTONE_1.md](docs/MILESTONE_1.md)

## Milestone 2 — Safe intake and snapshots

Status: implemented

- bounded folder and archive intake
- deterministic include/exclude inventory
- streaming file hashes and immutable source snapshots
- traversal, symlink, special-file, duplicate-path, depth, size, and archive
  expansion controls

Record: [docs/MILESTONE_2.md](docs/MILESTONE_2.md)

## Milestone 3 — Static source graph and generic audit

Status: implemented

- non-executing JSON, YAML, TOML, Rust, and Python scanners
- Python modules, functions, classes, imports, calls, tests, and dependency
  evidence
- scanner-neutral raw project graph
- duplicate, unresolved-reference, cycle, reachability, and provenance checks
- parser coverage, unsupported-format counts, skipped-check reasons, and
  language scan summary

Record: [docs/MILESTONE_3.md](docs/MILESTONE_3.md)

## Milestone 4 — Declarative biomedical adapter SDK

Status: implemented foundation; production adapter packages remain separate

- strict data-only JSON adapter packages
- detection, mapping, policy, fixture, and projection contracts
- bounded Rust interpreter with package/source/conformance identities
- detection, plan, semantic graph, semantic audit, and projection stages
- exact-hash approval gates and conformance checks

Record: [docs/MILESTONE_4.md](docs/MILESTONE_4.md)

## Milestone 5 — Safe local browser source intake

Status: implemented local developer workflow

- loopback-only Rust HTTP service with embedded UI
- browser folder and ZIP/TAR upload
- public GitHub repository import pinned to an immutable commit
- source-acquisition provenance and recent-run resume
- Host, same-origin, CSRF, body-limit, CSP, redirect, and no-CORS boundaries

Record: [docs/MILESTONE_5.md](docs/MILESTONE_5.md)

## Milestone 6 — Biomedical profile contract and profiler

Status: implemented in source; release verification required

- strict `qbm.benchmark-profile/v1` schema
- explicit `qbm.profile` JSON/YAML ingestion
- conservative structured biomedical heuristic
- evidence, diagnostics, confidence, defaults, assumptions, and approval state
- deterministic stable profile identity for compatible project revisions
- honest skip artifacts when biomedical semantics are unavailable

## Milestone 7 — Profile comparison

Status: implemented in source; release verification required

- latest earlier approved baseline selection by exact `profile_id`
- inputs, outcomes, and relationships added/removed
- reachability and active/inert state changes
- coverage changes at applicable K values
- minimum-panel changes at 80%, 90%, and 100% coverage
- QUBO size, coupling, and structural-difficulty changes

## Milestone 8 — Deterministic classical optimization

Status: implemented in source; release verification required

- structural reachability and density metrics
- weighted coverage at K and minimum-panel calculation
- bounded exhaustive exact search
- deterministic greedy, seeded simulated annealing, and seeded tabu search
- explicit optimality and limitation fields

## Milestone 9 — QUBO/Ising and optional quantum boundary

Status: logical formulation and provider-neutral contracts implemented;
provider execution is not implemented in the default workflow

- deterministic QUBO construction and scoring
- QUBO-to-Ising conversion
- bounded energy-equivalence validation
- variable/coupling/range metrics and structural difficulty estimate
- versioned provider capability, payload, request, job, result, and native
  rescoring contracts
- export-only default readiness with no endpoints or credentials

## Milestone 10 — Fourteen-stage biomedical browser workflow

Status: implemented in source; end-to-end release verification required

- manual exact-hash approval or rejection at every material checkpoint
- adapter stages connected to the browser and marked not applicable when no
  adapter is eligible
- profile, comparison, classical, and QUBO/Ising stages connected to the
  approved source lineage
- structured `qbm.full-report/v1` within `qbm.audit-bundle/v2`
- audit, semantic, optimization, comparison, quantum-formulation, limitation,
  and reproducibility sections

Architecture: [docs/BIOMEDICAL_WORKFLOW.md](docs/BIOMEDICAL_WORKFLOW.md)

## Remaining before a community release

The following work is intentionally not described as complete:

- repeatable end-to-end fixtures for folders, archives, and public GitHub
  across supported operating systems;
- production biomedical adapter/exporter packages for individual project
  families such as PATHEX or ONCOVA;
- immutable API-delta and local-bulk connectors;
- private-repository credential brokering and additional allowlisted Git hosts;
- background jobs, cancellation, progress streams, and crash recovery;
- isolated and explicitly approved behavior/model execution, if added at all;
- accessibility automation, fuzz/property/security testing, SBOM, signed
  artifacts, and cross-platform packaging;
- a stable external executor plugin protocol; and
- independently reviewed provider integrations, hardware embedding, result
  retrieval, and native biomedical rescoring.

The default local workflow will remain export-only for quantum execution until
a separately reviewed provider integration is intentionally configured.

## Release verification gates

Before tagging a public release, record results for:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
cargo deny check
```

Also verify folder, archive, and public GitHub UI flows; reload/resume; stale
approval rejection; complete and skipped profile paths; version comparison;
report download; and an event-chain integrity check. Do not convert a skipped
capability into a passing result in documentation, UI, or release notes.
