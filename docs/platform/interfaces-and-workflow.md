# Workflow, Approvals, CLI, API, and Browser UI

## 1. One workflow, three interfaces

The CLI, HTTP API, and browser UI are clients of the same Rust application layer.

```text
qbm CLI -----------+
                   |
browser UI -> HTTP API -> application services -> orchestrator/workers
                   |
automation client -+
```

No audit behavior may exist only in the UI. A run created in the browser can be inspected and approved from the CLI, and vice versa.

## 2. Primary user journeys

### 2.1 First-time project onboarding

1. Import a folder, archive, or manifest.
2. Review blocked, excluded, sensitive, and oversized files.
3. Approve the inventory.
4. Approve the immutable snapshot.
5. Review detected adapter candidates.
6. Review generated semantic mappings and unresolved items.
7. Approve policies, behavior runners, and projection catalog.
8. Run full audit.
9. Review findings and limitations.
10. Approve the baseline as `latest-approved`.

### 2.2 Later project update

1. Import or detect the changed project.
2. Build a new snapshot.
3. Apply the already approved adapter and policies.
4. Surface only additions, removals, ambiguous mappings, and policy changes.
5. Run structural and behavioral regression.
6. Compare against the previous approved snapshot.
7. Approve or reject promotion.

### 2.3 Audit without optimization

The user may stop after validation, structural audit, behavioral audit, and diff. Optimization and quantum stages are optional branches, not completion requirements.

### 2.4 What-if optimization

The user selects an approved projection and supplies or chooses an objective contract. Q-BenchMed runs classical baselines first. It produces candidate solutions and behavior checks in the audit store; it does not modify the project.

### 2.5 Quantum experiment

Only an approved abstract projection may enter this stage. The UI shows exactly which coefficients, circuit description, and metadata would leave the local machine. External submission requires an explicit approval tied to that payload hash.

## 3. Approval model

### 3.1 Approval object

```yaml
schema_version: qbm.approval/v1
approval_id: urn:qbm:approval:<uuid>
run_id: urn:qbm:run:<uuid>
stage: semantic-mapping
stage_output_hash: "<sha256>"
decision: APPROVE
actor:
  id: local-user
  role: project-owner
reason: "Mapping and exclusions reviewed"
created_at: "2026-08-30T12:00:00Z"
previous_event_hash: "<sha256>"
signature: null
```

The approval is valid only for the exact `stage_output_hash` and approval-policy version.

### 3.2 Decision semantics

#### APPROVE

Marks the stage output eligible for dependent stages. It does not make a clinical or scientific truth claim unless the approval profile explicitly assigns such a qualified role and meaning.

#### REJECT

Stops the run branch. Existing artifacts remain available. A new run or branch is required after source correction.

#### REQUEST_CHANGES

Returns the run to a named stage. The decision must contain a reason. Any dependent approvals become stale when outputs change.

#### WAIVE

Records accepted risk. It is disabled by default and cannot waive non-waivable policies such as hash mismatch, forbidden data export, or missing external-submission consent.

### 3.3 Approval profiles

Examples:

```text
regulated-onboarding
  manual approval: every material stage

regulated-continuous
  automatic: inventory when exclusions unchanged
  manual: semantics, behavior change, optimization, publish

software-ci
  automatic: static scan and generic audit
  manual: execute untrusted runner, publish baseline

research-quantum
  manual: projection, external payload, provider submission
```

### 3.4 Approval queue

Each approval request shows:

- what completed;
- exact input and output hashes;
- what changed from the last approved run;
- open findings by severity;
- requested capabilities used by the next stage;
- sensitive material involved;
- consequences of approve or reject;
- skipped or incomplete checks.

“Approve all” is permitted only for stages explicitly grouped by policy and must show the complete set of output hashes.

## 4. CLI specification

The executable is `qbm`.

### 4.1 Service and configuration

```bash
qbm init
qbm serve --bind 127.0.0.1:8765
qbm config show
qbm doctor
```

`qbm serve` binds to loopback by default. Listening on a non-loopback address requires explicit configuration and authentication.

### 4.2 Project intake

```bash
qbm project import /path/to/PATHEX
qbm project import /path/to/oncova.zip
qbm project import-manifest /path/to/.qbench/project.yaml
qbm project list
qbm project show PATHEX
```

Useful options:

```text
--name <display-name>
--adapter <adapter-id>
--approval-profile <profile-id>
--include <glob>
--exclude <glob>
--manifest-only-large-artifacts
--no-network
```

Intake never follows an include flag into a path denied by global security policy.

### 4.3 Runs

```bash
qbm run start PATHEX --mode onboarding
qbm run start ONCOVA --mode continuous --baseline latest-approved
qbm run list --project ONCOVA
qbm run status <run-id>
qbm run logs <run-id> --stage structural-audit
qbm run resume <run-id>
qbm run cancel <run-id>
```

Cancellation is cooperative and preserves completed immutable artifacts.

### 4.4 Approval commands

```bash
qbm run approvals <run-id>
qbm run approve <run-id> --stage inventory --comment "Reviewed exclusions"
qbm run reject <run-id> --stage semantic-mapping --reason "Pregnancy misclassified as selectable"
qbm run request-changes <run-id> --stage semantic-mapping \
  --return-to adapter --reason "Use oncology policy v2"
```

The CLI prints the stage hash and summary before accepting an interactive approval. Automation must pass `--expected-hash`; this prevents approving a changed artifact accidentally.

### 4.5 Snapshot and diff

```bash
qbm snapshot list ONCOVA
qbm snapshot show <snapshot-id>
qbm snapshot promote <snapshot-id> --to validated
qbm diff ONCOVA --from latest-approved --to latest-discovered
qbm diff export <diff-id> --format json
```

Promotion still checks the configured approval policy; the command is not an override.

### 4.6 Audit

```bash
qbm audit start ONCOVA --snapshot latest-approved
qbm audit findings <run-id> --severity high
qbm audit explain <finding-id>
qbm audit export <run-id> --format bundle
```

### 4.7 Optimization

```bash
qbm projection list ONCOVA
qbm optimize start <projection-id> --objective objective.yaml --backend exact-ilp
qbm optimize compare <run-id> --backends exact-ilp,greedy,annealing,tabu
qbm optimize explain <solution-id>
```

Every optimization command shows whether the projection is approved, whether behavior-preservation tests exist, and whether the result is exact, bounded, or heuristic.

### 4.8 Quantum

```bash
qbm quantum inspect <projection-id>
qbm quantum prepare <projection-id> --backend local-simulator
qbm quantum prepare <projection-id> --backend ibm --depth 1 --shots 2048
qbm run approve <run-id> --stage external-quantum-payload
qbm quantum submit <run-id>
```

`prepare` freezes the exact external payload. `submit` refuses if its hash differs from the approved payload.

### 4.9 Reports

```bash
qbm report build <run-id>
qbm report show <run-id>
qbm report open <run-id>
qbm report export <run-id> --format html --output /approved/path
```

## 5. HTTP API

The initial API is local and versioned under `/api/v1`.

### 5.1 Project and upload routes

```text
POST   /api/v1/projects
GET    /api/v1/projects
GET    /api/v1/projects/{project_id}
POST   /api/v1/uploads
GET    /api/v1/uploads/{upload_id}
POST   /api/v1/projects/{project_id}/snapshots
GET    /api/v1/projects/{project_id}/snapshots
```

Uploads are streamed. The server does not buffer the entire project in memory.

### 5.2 Run routes

```text
POST   /api/v1/projects/{project_id}/runs
GET    /api/v1/runs/{run_id}
POST   /api/v1/runs/{run_id}/resume
POST   /api/v1/runs/{run_id}/cancel
GET    /api/v1/runs/{run_id}/stages
GET    /api/v1/runs/{run_id}/events
GET    /api/v1/runs/{run_id}/stream
```

`/stream` uses server-sent events for progress. WebSockets are unnecessary for the first version.

### 5.3 Approval routes

```text
GET    /api/v1/runs/{run_id}/approvals
POST   /api/v1/runs/{run_id}/stages/{stage_id}/decisions
```

Decision payload:

```json
{
  "expected_stage_output_hash": "...",
  "decision": "APPROVE",
  "reason": "Reviewed",
  "return_to_stage": null
}
```

Hash mismatch returns a conflict response and requires the UI to refresh.

### 5.4 Findings, graphs, diffs, and reports

```text
GET /api/v1/runs/{run_id}/findings
GET /api/v1/findings/{finding_id}
GET /api/v1/runs/{run_id}/graphs/{graph_kind}
GET /api/v1/runs/{run_id}/diff
GET /api/v1/runs/{run_id}/optimization
GET /api/v1/runs/{run_id}/report
GET /api/v1/runs/{run_id}/artifacts/{artifact_id}
```

Artifact endpoints enforce sensitivity and safe media rendering; unknown binary content is downloaded, not rendered inline.

## 6. Minimal browser UI

### 6.1 Technology choice

Use a Rust web server and a Rust/WASM frontend where practical. The recommended initial composition is:

```text
Axum/Tokio HTTP service
Rust application/domain crates
Leptos or equivalent Rust/WASM UI
SSE for progress
embedded static assets in release builds
```

A small JavaScript interoperability shim is acceptable for browser-only features such as directory selection or file-system APIs. Business logic remains in Rust.

### 6.2 Folder import reality

A normal browser cannot silently read an arbitrary local directory. Support three user-safe choices:

1. directory chooser using a browser directory-upload control;
2. ZIP/TAR archive upload;
3. local-path registration through the CLI or desktop-local server.

For very large ONCOVA-style corpora, local-path or manifest import is preferred. Uploading hundreds of gigabytes through the browser is unnecessary and inefficient.

### 6.3 Screens

#### Home / Projects

- project cards;
- latest discovered/validated/approved versions;
- open critical findings;
- last run status;
- `Import project` action.

#### Import wizard

- folder/archive/path/manifest selection;
- live file count and byte total;
- detected secret/sensitive-file warnings;
- inclusion/exclusion preview;
- adapter candidates;
- start onboarding.

#### Run timeline

```text
Inventory        COMPLETE       [View]
Snapshot         WAITING        [Approve] [Reject]
Adapter          PENDING
Semantic graph   PENDING
Audit            PENDING
Report           PENDING
```

The selected stage shows inputs, output hash, logs, findings, artifact links, requested next-stage capabilities, and approval controls.

#### Semantic mapping review

- discovered entity;
- proposed kind;
- confidence;
- source evidence;
- previous mapping;
- conflict indicator;
- bulk approve unchanged mappings;
- resolve only new/ambiguous mappings.

#### Findings explorer

- severity/category/status filters;
- evidence and trace path;
- impact explanation;
- recommended action;
- affected versions and projections;
- accepted-risk or false-positive workflow when policy permits.

#### Graph explorer

- choose graph layer;
- search node;
- expand neighbors on demand;
- highlight unreachable, cyclic, changed, or policy-violating nodes;
- show raw-to-semantic-to-projection trace;
- avoid rendering the entire huge graph at once.

#### Version diff

- summary counts;
- added/removed/renamed/changed entities;
- changed relationships and condition groups;
- reachability and behavior changes;
- source and model version changes;
- risk classification.

#### Optimization

- projection selector;
- objective and constraint form;
- classical baseline selection;
- solution comparison;
- coverage and cost curves;
- protected-invariant status;
- optional quantum eligibility and resource estimate.

#### Report

- executive summary;
- completeness/limitations banner;
- findings and evidence;
- diff, behavior, and optimization sections;
- approval history;
- download JSON/CSV/Markdown/HTML bundle.

## 7. Automation triggers

Supported triggers:

- manual UI/CLI start;
- filesystem watch after explicit registration;
- Git hook or CI call;
- scheduled connector refresh;
- API webhook into quarantine;
- new approved bulk-release manifest.

Triggers create discovered snapshots. They do not automatically promote them to approved status.

## 8. Notification behavior

The local UI should notify only for:

- approval required;
- stage failure;
- new critical/high finding;
- unexpected sensitive content;
- completed report;
- external connector or provider action requiring consent.

Routine scan events belong in the timeline, not as disruptive notifications.

## 9. Accessibility and usability requirements

- Every UI action has a CLI/API equivalent.
- Severity is never communicated by color alone.
- Approval dialogs use plain language and show exact consequences.
- Long jobs expose progress and may be resumed.
- Findings link directly to source evidence when locally available.
- Reports explain the difference between structural coverage, behavioral preservation, and scientific validity.
- The UI never labels a heuristic answer as “optimal.”

## 10. End-to-end acceptance scenario

For ONCOVA:

1. User selects the ONCOVA folder.
2. UI inventories GUARD, CORPUS, manifests, schemas, scripts, and documentation.
3. Sensitive/excluded artifacts are displayed.
4. User approves inventory and snapshot.
5. `oncova` adapter is detected and approved.
6. Raw graphs and proposed semantic mappings are built.
7. User approves unchanged known mappings and resolves any new ambiguity.
8. Static GUARD/CORPUS/pipeline audits run.
9. The approved local gate runners execute without network.
10. The new result is compared with the previous approved ONCOVA snapshot.
11. Safe benchmark projections are built.
12. Optional classical optimization runs.
13. A report package is created.
14. User approves or rejects promotion of the snapshot and report.

At no point is ONCOVA source code modified or a clinical claim promoted automatically.

