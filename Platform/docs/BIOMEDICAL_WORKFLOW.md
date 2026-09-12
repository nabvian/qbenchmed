# Biomedical Workflow and Architecture

This document describes Q-BenchMed Platform as **A Modular Open-Source
Quantum–Classical Benchmarking Framework for Biomedical Input Optimization**.
It is the implementation guide for users, profile exporters, adapter authors,
and optional quantum-executor integrators.

## The question Q-BenchMed answers

Given a reviewed set of biomedical inputs, reviewed biomedical outcomes, and a
reviewed map of which input can cover which outcome, Q-BenchMed asks:

> Which feasible input panel gives the best weighted outcome coverage under the
> declared size, cost, inclusion, exclusion, and required-outcome constraints?

Examples of an input include an assay, biomarker, feature, rule input, imaging
sequence, or experimental measurement. Examples of an outcome include a
declared clinical decision, phenotype, molecular state, or coverage target.
Those meanings come from the profile and its evidence. Q-BenchMed never assigns
them by interpreting arbitrary source-code names.

## Architecture at a glance

```text
folder / archive / public GitHub repository
                    |
                    v
          bounded source acquisition
                    |
                    v
    inventory -> immutable source snapshot
                    |
                    v
      static scanners -> scanner-neutral graph
                    |
            +-------+-------+
            |               |
            v               v
   generic source audit   declarative biomedical adapters
                            (when eligible)
            |               |
            +-------+-------+
                    v
       explicit profile or conservative profiler
                    |
                    v
         approved qbm.benchmark-profile/v1
              /             |             \
             v              v              v
      version comparison  classical      QUBO -> Ising
                          baselines       validation
              \             |             /
               +------------+------------+
                            v
             full report + reproducibility chain
```

The trust boundary deliberately narrows as data moves down the diagram. The
source layer records what exists. An adapter may interpret frozen evidence. The
profile states exactly what can be optimized. Numerical engines consume only a
validated profile.

## Component responsibilities

| Component | Responsibility | Boundary |
| --- | --- | --- |
| `qbm-intake` | Folder/archive inventory and immutable snapshots | Never modifies the source tree |
| `qbm-audit` | JSON/YAML/TOML/Rust/Python static scanning and generic findings | Never imports, compiles, or executes project code |
| `qbm-adapter` | Bounded interpretation of raw evidence through declarative packages | No executable plugins or host permissions |
| `qbm-profiler` | Explicit profile decoding or conservative structured biomedical projection | JSON/YAML values only; every heuristic candidate requires approval |
| `qbm-benchmark` | Validation, metrics, profile diff, classical solvers, QUBO/Ising construction | Pure Rust; no filesystem, database, web, or provider dependency |
| `qbm-quantum` | Versioned provider-neutral contracts, energy validation, and native rescoring boundary | No endpoints, credentials, provider SDK, or built-in network execution |
| `qbm-app` | Durable artifacts, approvals, event chains, and stage lineage | Exact predecessor approval is required |
| `qbm-web` | Loopback UI, source acquisition, stage orchestration, and full report | Same-origin, CSRF, Host, body-size, and CSP controls |

## Source acquisition and static analysis

The browser accepts three source kinds:

1. A browser-selected folder. Relative paths and file bytes are copied into
   managed storage; an absolute client path is not trusted as an API argument.
2. A ZIP, TAR, TAR.GZ, or TGZ archive. A frozen archive copy is inspected with
   path traversal, symlink, special-file, duplicate-path, depth, size, and
   expansion limits.
3. A public GitHub repository-root URL. A branch or tag is first resolved to a
   full commit and the bounded codeload artifact is hashed.

The immutable snapshot connects every later observation to exact source bytes.
The scanner then creates a language-neutral graph:

- JSON, YAML, and TOML scanners record structured values and references;
- the Rust scanner records syntax structure without compiling it; and
- the Python scanner records modules, classes, functions, imports, calls,
  test-like definitions, and dependency evidence without importing modules.

Parser coverage distinguishes eligible, parsed, unsupported, and skipped files
and bytes. Generic checks cover only what their evidence supports, such as
duplicates, unresolved references, cycles, reachability, and provenance.
Dynamic dispatch, generated code, runtime imports, macros, foreign-function
effects, model behavior, API responses, and test results are not established by
static scanning.

## Declarative biomedical semantics

An installed adapter is an immutable JSON package interpreted by trusted Rust
code. It can declare detection rules, node/relationship mappings, semantic
policies, projections, and synthetic conformance fixtures. It cannot execute a
project, start a process, load a native library, read arbitrary files, or use
the network.

If no adapter is eligible, the adapter plan, semantic graph, semantic audit,
and projection-catalog stages are marked not applicable. That does not prevent
an explicit benchmark profile from being validated. Conversely, an adapter
projection does not automatically prove that a profile is correct; the profile
still has its own evidence, assumptions, validation, and approval checkpoint.

See [Declarative adapter authoring](ADAPTER_AUTHORING.md) for the package
contract.

## The `qbm.profile` contract

The canonical schema identifier is `qbm.benchmark-profile/v1`. The exact
filenames `qbm.profile.json`, `qbm.profile.yaml`, and `qbm.profile.yml` (and
their `qbm-profile` spellings) force explicit-profile interpretation. A document
with the benchmark schema marker is also explicit under another filename. JSON
and YAML are supported; an explicit payload can be the root object or a
recognized `qbm.profile`, `qbm-profile`, or `profile` envelope.

```json
{
  "schema_version": "qbm.benchmark-profile/v1",
  "profile_id": "biomedical/nsclc-panel",
  "title": "NSCLC demonstration panel",
  "biomedical_scope": {
    "area": "oncology",
    "population": "Demonstration NSCLC cohort",
    "input_semantics": "A selectable biomarker assay",
    "outcome_semantics": "A declared decision-support coverage target"
  },
  "inputs": [
    {"id": "assay.egfr", "label": "EGFR assay", "cost": 1.0, "tags": ["genomic"]}
  ],
  "outcomes": [
    {"id": "target.egfr", "label": "EGFR target covered", "weight": 1.0, "tags": ["therapy"]}
  ],
  "relationships": [
    {"input_id": "assay.egfr", "outcome_id": "target.egfr"}
  ],
  "constraints": {
    "min_selected": 0,
    "max_selected": 1,
    "max_total_cost": null,
    "required_inputs": [],
    "excluded_inputs": [],
    "required_outcomes": []
  },
  "objective": "maximize_weighted_coverage",
  "provenance": {
    "generated_by": "example-exporter/v1",
    "source_revision": "demo-v1",
    "source_artifact_ids": ["example:nsclc:v1"],
    "projection_method": "explicit reviewed export"
  }
}
```

The complete example is
[`examples/profiles/nsclc-mini.qbm.profile.json`](../examples/profiles/nsclc-mini.qbm.profile.json).

### Identity and ordering rules

- Keep `profile_id` unchanged for successive versions of one logical
  biomedical optimization problem. Change it when the population, semantics,
  or intended problem identity changes materially.
- Input and outcome IDs must remain stable across versions. Labels may change
  without hiding identity continuity.
- IDs use ASCII letters, digits, `.`, `_`, `:`, `/`, or `-` and contain at most
  256 bytes.
- Inputs and outcomes are strictly sorted by ID. Relationships are strictly
  sorted and unique by `(input_id, outcome_id)`.
- Tags, required/excluded input IDs, required outcome IDs, and source artifact
  IDs are strictly sorted and unique.
- Costs and weights must be finite and strictly positive.
- Every relationship and constraint reference must resolve to a declared ID.
- Required and excluded inputs cannot overlap. Required outcomes must remain
  reachable using non-excluded inputs.
- Unknown fields are rejected; the schema does not silently ignore typos.

Cost is intentionally unit-neutral, but every input in one profile must use the
same reviewed unit or normalization. Outcome weights express declared relative
importance; they are not probabilities and Q-BenchMed does not infer them.

### Heuristic projection

When no explicit profile is present, `qbm-profiler` can inspect a bounded set of
parsed JSON/YAML documents for a small set of structured biomedical rule and
registry shapes. The result is a projection candidate, never an automatic
truth claim. It includes:

- source artifact IDs, file paths, and JSON Pointers for extracted fields;
- a confidence value;
- diagnostics and ignored/ambiguous evidence;
- every defaulted field and assumption; and
- a `requires_approval` status.

The browser currently considers at most 128 structured documents and at most
16 MiB per document for profiling, with additional depth, element, string,
input, outcome, relationship, and evidence ceilings. Files with names that
contain `qbm.profile` receive collection priority, but the schema/envelope rules
above determine whether a document is explicit. If projection is unsupported
or ambiguous, add an explicit exporter rather than renaming source-code symbols
to look biomedical.

## Version comparison

After approving a profile, Q-BenchMed searches prior runs for the latest
approved profile with the same `profile_id`. It never compares profiles merely
because folder names look similar. The comparison contains:

- input IDs added and removed;
- outcome IDs added and removed;
- relationships added and removed;
- newly unreachable and newly reachable outcomes;
- inputs whose inert/active state changed;
- weighted coverage changes at applicable K values among 5, 10, 20, 50, and
  100;
- minimum-panel changes for 80%, 90%, and 100% coverage; and
- QUBO variable-count, coupling-count, and structural-difficulty changes under
  a common feasible panel ceiling.

Some before/after values can be unavailable when K exceeds a profile's input
count or a constraint makes the requested result infeasible. The report retains
that absence or error rather than converting it to zero.

## Classical optimization

The objective is maximum weighted coverage. A selected input covers each linked
outcome at most once, even if several selected inputs link to it. The score also
reports selected count, total cost, covered and uncovered outcomes, covered
weight, coverage fraction, and constraint violations.

The browser computes coverage at applicable K values and minimum panels for
80%, 90%, and 100% weighted coverage. It compares deterministic greedy,
simulated-annealing, and tabu-search baselines. Exact enumeration is included
only when the profile has at most 20 inputs and the search remains within
`2^20` states.

Only an exact result that completed within its declared bound proves optimality.
Greedy, annealing, and tabu results can be compared reproducibly because their
seeds and iteration counts are recorded, but they remain heuristics.

### Audit drill-down and wiring terminology

The browser and approved exports retain the full records behind each summary:

- every input ID, label, cost, tag, and linked outcome;
- every outcome ID, label, weight, tag, and supporting input;
- every approved binary input–outcome relationship; and
- for every measured K, the selected inputs, covered outcomes, uncovered
  outcomes, covered weight, total cost, constraint status, solver metadata,
  limitations, and optimality flag.

`K` is a maximum number of selected inputs. It is not a requested number of
outcomes and a solver may select fewer than K when no remaining input adds
approved weighted coverage.

An **inert input** or **unreachable outcome** means that no relationship in the
approved `qbm.profile` connects that record. It is deliberately not phrased as
“unused in the source project.” Static scanning can miss dynamic logic,
generated code, unsupported formats, runtime API behavior, or meanings that
require an adapter. The wiring diagnostic therefore includes the projection
method, confidence, assumptions, diagnostics, semantic-layer status, and this
interpretation boundary alongside the expanded lists.

## QUBO, Ising, and optional quantum execution

The formulation stage builds a logical binary QUBO from the same approved
profile and optimization request, converts it to an Ising model, validates
dimensions/order/finite coefficients, and checks energy equivalence over a
bounded deterministic assignment set. It reports:

- logical variables and their source bindings;
- linear and quadratic terms;
- model offset and constraint penalty;
- variable and coupling counts and coefficient ranges; and
- a structural difficulty level and score.

Difficulty is a transparent structural heuristic. It does not predict wall
time, solution quality, embeddability on a particular device, or quantum
advantage.

The default local browser flow is **export-only**. `qbm-quantum` defines the
versioned contracts needed by a future or external executor—approved payload,
backend capability, request, receipt, status, result, assignment convention,
and native rescoring—but performs no provider I/O itself. A provider integration
must be separately implemented and reviewed, bind execution to the exact
approved model hash, keep credentials outside artifacts, and rescore returned
samples in the native biomedical model.

After approval, the browser exposes the validated logical formulation as
`qbm.quantum-formulation-export/v1`. This portable file includes the exact
stage-output identity, approval lineage, QUBO, Ising model, equivalence evidence,
difficulty estimate, and the explicit no-advantage interpretation boundary. It
is not itself a provider submission receipt.

## Approval and reproducibility model

Every producer requires the exact approved predecessor output. Each approval
records the run, stage, decision, actor, expected output hash, time, and optional
reason. The application rejects stale approvals and records a tamper-evident
per-run event chain.

The final report assembles section status, artifact hashes, acquisition
provenance, parser coverage, language scan, findings, profile evidence,
comparison, optimization, QUBO/Ising validation, approvals, events, and solver
seeds. A `skipped` or `not_applicable` section is a reproducible result with a
reason; it must not be presented as a successful analysis.

## Limitations to communicate to users

- Static analysis is evidence about source structure, not runtime behavior.
- Unsupported formats reduce parser coverage and can make checks unavailable.
- An adapter encodes reviewed interpretation rules; it does not establish
  biomedical truth.
- A heuristic profile is a review candidate and may be incomplete.
- Optimization is valid only for the declared relationships, costs, weights,
  constraints, and objective.
- Exact optimality is bounded; heuristic solvers do not prove it.
- QUBO/Ising equivalence is logical-model validation, not provider execution.
- Nothing in a report establishes safety, efficacy, diagnosis, regulatory
  compliance, or quantum advantage.
