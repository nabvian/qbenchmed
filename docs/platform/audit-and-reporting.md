# Audit, Optimization, Diff, and Report Contracts

## 1. Audit principle

An audit is a reproducible set of checks executed against immutable artifacts. Its result is not a single score. It is a structured collection of findings, measurements, limitations, skipped checks, evidence, and approval decisions.

Q-BenchMed must always answer:

- What exact project version was inspected?
- What was included and excluded?
- Which adapter and policies interpreted it?
- Which checks ran, failed, passed, or were skipped?
- What evidence supports each finding?
- What behavior changed from the baseline?
- What can and cannot be concluded?
- Which human decisions authorized the next stage?

## 2. Audit families

### 2.1 Intake and inventory audit

- inaccessible or unreadable artifacts;
- unsafe archive entries or symlinks;
- unexpected binary/large files;
- excluded paths and exclusion reasons;
- probable secrets or sensitive data;
- duplicate content;
- missing project version;
- dirty or incomplete Git state;
- unsupported formats.

### 2.2 Structural audit

- duplicate IDs;
- missing and invalid references;
- invalid type relations;
- cycles where a DAG is required;
- unreachable targets;
- inert or unused inputs;
- disconnected subgraphs;
- rules without outputs;
- outputs without satisfiable paths;
- malformed condition groups;
- duplicate and dominated paths;
- stage-order and dependency violations;
- untested components;
- documentation-to-implementation mismatches where both are machine identifiable.

### 2.3 Semantic-policy audit

Examples:

- selectable versus context versus derived-variable confusion;
- missing safety classification;
- missing domain authority or approval state;
- invalid cross-boundary link;
- research object leaking into a production/clinical projection;
- machine-proposed evidence presented as approved;
- forbidden objective or optimizer target;
- changed threshold or priority without required provenance.

### 2.4 Provenance, licence, and reproducibility audit

- missing source identity or hash;
- missing version/access date;
- missing source span;
- unresolved or incompatible licence;
- stale/superseded source still active;
- missing adapter/policy/engine version;
- nondeterministic output without seed/repetition metadata;
- report artifact not traceable to snapshot;
- external API result not mirrored locally.

### 2.5 Behavioral audit

- golden test failures;
- old/new output differences;
- safety-state loss;
- missing-data behavior change;
- subgroup or slice regression;
- retrieval recall/precision change;
- index-versus-scan mismatch;
- API fixture/contract incompatibility;
- model metric, calibration, latency, memory, or shape regression;
- nondeterministic output outside declared tolerance.

### 2.6 Security and privacy audit

- secrets in snapshot or report;
- protected data in exportable projection;
- unapproved network capability;
- unsafe runner contract;
- executable or macro-enabled files not quarantined;
- archive/path abuse;
- dependency lock or signature problem;
- report content capable of active execution;
- external quantum payload containing forbidden data.

### 2.7 Optimization-readiness audit

- missing decision-variable classification;
- missing objective;
- missing costs/weights or units;
- hard constraint treated as soft;
- safety invariant absent;
- backend unable to represent conjunction, precedence, negation, or conditional constraints;
- missing evaluation/behavior-preservation data;
- objective creates a harmful proxy;
- projection includes non-approved semantics.

### 2.8 Quantum-readiness audit

- exact QUBO equivalence not verified;
- excessive variable/qubit count;
- penalty scale or dynamic-range problem;
- bit-order/decode map missing;
- native rescoring missing;
- no exact/small-instance classical reference;
- circuit/resource estimate absent;
- external payload not approved;
- privacy or IP policy violation;
- quantum result compared unfairly with a different classical problem.

## 3. Finding schema

```json
{
  "schema_version": "qbm.finding/v1",
  "finding_id": "QBM-STRUCT-BROKEN-REF-<stable-hash>",
  "run_id": "urn:qbm:run:...",
  "rule_id": "generic.reference-integrity/v1",
  "severity": "HIGH",
  "confidence": "CONFIRMED",
  "category": "broken_reference",
  "status": "OPEN",
  "title": "Derived feature references an undeclared input",
  "message": "LDH_FOLD_ULN references LDH_ULN, which is not declared in this snapshot.",
  "impact": "The dependent rule path cannot be evaluated.",
  "affected": ["semantic:derived-feature:LDH_FOLD_ULN"],
  "evidence": [
    {
      "kind": "source_span",
      "artifact_hash": "...",
      "relative_path": "knowledge/formulas.yaml",
      "line_start": 42,
      "line_end": 44
    }
  ],
  "recommended_action": "Declare the referenced metadata input or correct the formula reference.",
  "automatic_fix_available": false,
  "introduced_in": "snapshot:...",
  "baseline_status": "NEW",
  "policy_tags": ["structure", "evaluability"]
}
```

Finding IDs are stable for the same rule and semantic subject so users can track resolution across versions. Message text may improve without creating a new identity.

## 4. Severity and confidence

### Severity

| Severity | Meaning |
|---|---|
| CRITICAL | Invalidates a required safety, integrity, privacy, or audit boundary; promotion blocked |
| HIGH | Likely material failure or major coverage/behavior/provenance gap |
| MEDIUM | Real defect or risk with bounded impact |
| LOW | Limited defect, maintainability problem, or weak assurance |
| INFO | Observation, opportunity, or expected limitation |

Severity is impact, not certainty.

### Confidence

| Confidence | Meaning |
|---|---|
| CONFIRMED | Direct deterministic evidence or failed assertion |
| HIGH | Strong inference from multiple consistent facts |
| MEDIUM | Plausible automated interpretation requiring review |
| LOW | Candidate or heuristic signal |

Potential duplicate, rename, dominance, and shadow findings normally begin below `CONFIRMED` until semantics or behavior tests establish equivalence.

## 5. Finding lifecycle

```text
OPEN
  -> ACKNOWLEDGED
  -> RESOLVED
  -> ACCEPTED_RISK
  -> FALSE_POSITIVE
  -> DEFERRED
```

State transitions are append-only review events. `ACCEPTED_RISK` and `FALSE_POSITIVE` require a reason, actor, snapshot scope, and expiry/review policy. A new materially changed finding is reopened.

## 6. Check result schema

Every planned check must appear in the report, including checks that did not run.

```yaml
check_id: oncova.guard.stage-order/v1
status: PASS       # PASS | FAIL | ERROR | SKIPPED | NOT_APPLICABLE
input_hashes: ["..."]
engine_version: qbm-audit/1.0
duration_ms: 12
findings: []
skip_reason: null
limitations: []
```

This prevents “no finding” from being mistaken for “check completed successfully.”

## 7. Version diff contract

The diff engine compares two immutable snapshots and their derived artifacts.

### 7.1 Change classes

```text
BREAKING
SAFETY_RELEVANT
BEHAVIORAL
ADDITIVE
REMOVAL
RENAME_CANDIDATE
STRUCTURAL
DATA_OR_MODEL
METADATA_ONLY
PROVENANCE_ONLY
```

A change may have multiple tags.

### 7.2 Diff dimensions

- inventory files and content hashes;
- connector receipts and upstream releases;
- raw nodes/edges;
- semantic nodes/edges/condition groups;
- policies and approvals;
- model and dataset identity;
- API/database schemas;
- projection membership;
- coverage/reachability metrics;
- behavior outputs and metrics;
- optimization result curves;
- QUBO variable and coefficient changes;
- report/check completeness.

### 7.3 Rename handling

The engine may propose a rename based on similarity, provenance, or stable aliases. Until approved, it reports one removal, one addition, and one `RENAME_CANDIDATE`; it does not silently merge identity.

### 7.4 Impact traversal

For each changed node, the engine traces affected downstream rules, targets, tests, models, projections, and reports. The impact trace records relation types and stops at configurable boundaries.

Example summary:

```text
Inputs:                  +8 / -1 / 2 changed
Outcomes:               +12 / -0 / 3 changed
Rule paths:             +21 / -4 / 9 changed
New unreachable targets: 1
Behavior changes:         7 of 10,000 fixtures
Safety regressions:       0
Coverage at K=10:       50 -> 57
Minimum full panel:     37 -> 41
```

## 8. Objective contract

No optimizer runs without a validated contract.

```yaml
schema_version: qbm.objective/v1
id: pathex.minimum-panel-preserve-behavior/v1
projection: pathex.behavior-preservation/v3

decision_variables:
  selector: selectable-inputs
  binary: true

objective:
  sense: minimize
  expression: total_input_cost

hard_constraints:
  - behavior_agreement >= 0.995
  - safety_output_recall == 1.0
  - required_context_preserved == true

soft_terms:
  - missing_data_failures

evaluation:
  fixture_set: pathex-golden-10000/v2
  subgroup_metrics: true

allowed_backends:
  - exact_ilp
  - greedy
  - annealing
  - tabu

result_use: research_only
```

The contract distinguishes:

- coverage of graph relationships;
- evaluability of outputs;
- actual behavioral preservation on fixtures;
- scientific or clinical sufficiency.

The report must not merge these concepts.

## 9. Optimization workflow

```text
Approved semantic graph
  -> approved projection
  -> objective validation
  -> protected-invariant validation
  -> classical exact/baseline runs
  -> heuristic runs
  -> native rescoring
  -> behavioral verification
  -> optional QUBO construction
  -> optional quantum experiment
  -> comparison and limitations
```

Every candidate solution is rescored using the backend-neutral native objective. A QUBO energy, solver-internal score, or quantum sample is not accepted as final truth.

## 10. Algorithm comparison

Results are comparable only when algorithms receive the same approved projection, objective, constraints, and evaluation method.

Report fields include:

- backend and version;
- exact/heuristic status;
- random seed and repetitions;
- wall-clock and CPU time;
- memory;
- objective value;
- feasibility;
- certified gap/bound where available;
- selected variables;
- achieved target coverage;
- protected-invariant results;
- behavior metrics;
- failure/timeout reason.

The report must say when a graph-native algorithm and a QUBO algorithm operate in different mathematical spaces.

## 11. Quantum report requirements

For local or external quantum runs:

- exact QUBO and Ising hashes;
- variable and bit-order maps;
- logical and physical qubit counts;
- circuit depth and gate counts after compilation;
- shots, QAOA depth, parameters, optimizer, and seeds;
- backend/provider identity;
- job ID and timestamps;
- calibration reference;
- mitigation/suppression settings;
- raw sample artifact;
- decoded candidate;
- native objective and feasibility;
- exact/small-instance or strongest classical reference;
- data-export declaration;
- explicit statement that hardware use is not evidence of advantage.

## 12. Report package

Each completed run creates:

```text
report-bundle/
├── manifest.json
├── REPORT.md
├── report.html
├── report.json
├── executive-summary.json
├── source/
│   ├── snapshot.json
│   ├── inventory.json
│   ├── connector-receipts.json
│   └── exclusions.json
├── graphs/
│   ├── raw-graph.json
│   ├── semantic-graph.json
│   ├── graph-summary.json
│   └── projections/
├── audit/
│   ├── checks.json
│   ├── findings.json
│   ├── findings.csv
│   ├── passed-checks.json
│   └── limitations.json
├── behavior/
│   ├── run-records.json
│   ├── regressions.csv
│   └── metrics.csv
├── diff/
│   ├── diff.json
│   ├── diff.csv
│   └── impact.json
├── optimization/
│   ├── objective.yaml
│   ├── runs.json
│   ├── solutions.csv
│   └── coverage-curve.csv
├── quantum/
│   ├── prepared-job.json
│   ├── provider-receipt.json
│   └── samples.json
├── governance/
│   ├── approvals.jsonl
│   ├── policies.json
│   └── capability-log.json
├── environment.json
└── SHA256SUMS
```

Folders for stages that did not run still contain a status/limitations record or are listed explicitly as absent in `manifest.json`.

## 13. Human-readable report structure

`REPORT.md` and `report.html` use this order:

1. **Executive conclusion** — promotion recommendation and most important risks.
2. **Scope and identity** — project, snapshot, adapter, policy, baseline.
3. **Completeness** — what ran, did not run, and why.
4. **Critical/high findings** — evidence and impact.
5. **Project structure** — inventory and graph summary.
6. **Semantic integrity** — mapping and policy results.
7. **Behavior** — fixtures, regressions, performance.
8. **Version diff** — changes and downstream impact.
9. **Optimization** — objective, solutions, feasibility, protected invariants.
10. **Quantum research** — only when run.
11. **Provenance, privacy, and licences**.
12. **Recommended actions** — ordered by severity, dependency, and effort class.
13. **Approval history**.
14. **Limitations and non-claims**.
15. **Reproduction instructions**.

## 14. Structured recommendations

Recommendations are separate from findings. One remediation may address several findings.

```yaml
recommendation_id: REC-ONCOVA-012
title: Add per-gene contraindication coverage
priority: P1
addresses_findings:
  - QBM-CORPUS-VETO-...
preconditions:
  - qualified domain review
actions:
  - author missing claims using approved template
  - attach primary-source provenance
  - run corpus gates
  - rerun Q-BenchMed diff
verification:
  - zero required veto gaps
  - no regression in approved retrieval fixtures
automatic_change: forbidden
```

Effort may be classified as `SMALL`, `MEDIUM`, `LARGE`, or `UNKNOWN`; do not invent calendar estimates without project evidence.

## 15. Project score policy

A single universal “health score” is optional and must never replace findings. If implemented, component scores remain visible:

```text
Structural completeness
Provenance completeness
Behavioral test completeness
Policy compliance
Reproducibility
Optimization readiness
```

Clinical validity, scientific truth, and safety approval are not computed scores.

## 16. ONCOVA report sections

An ONCOVA report adds:

- GUARD bundle identity and executable status;
- safety-stage ordering and reachability;
- required/missing/confirmatory test coverage;
- conflict and kill-gate coverage;
- CORPUS claim counts by review state and scope;
- per-scope and per-driver veto gaps;
- source/licence/hash completeness;
- retrieval regression and index/scan parity;
- corpus snapshot change impact;
- strict separation of GUARD, reviewed corpus, and research graph;
- explicit reminder that engineering audit does not change `DRAFT_NON_EXECUTABLE` or clinical-validation status.

## 17. PATHEX report sections

A PATHEX report adds:

- declared, reachable, and behavior-tested outcomes;
- selectable, context, and derived input classification;
- unit/normalization consistency;
- rule-path conjunction preservation;
- inert and marginal-value inputs;
- coverage curve by input budget;
- minimum panel for declared coverage;
- minimum panel that preserves behavior on approved fixtures;
- safety-output preservation;
- previous-version reconciliation;
- difference between mathematical evaluability and clinical sufficiency.

## 18. Promotion recommendation

The report may recommend:

```text
APPROVE_BASELINE
APPROVE_WITH_ACCEPTED_RISK
REQUEST_CHANGES
REJECT_PROMOTION
INCOMPLETE_AUDIT
```

This is a policy-derived workflow recommendation. Only a recorded human decision changes an approved pointer.

