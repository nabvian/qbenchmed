# Q-BenchMed Platform

**A Modular Open-Source Quantum–Classical Benchmarking Framework for
Biomedical Input Optimization.**

Q-BenchMed turns an immutable snapshot of a biomedical project into a reviewed
input/outcome profile, measures how efficiently candidate input panels cover
declared outcomes, compares approved profile versions, and validates a portable
QUBO/Ising formulation. The implementation is Rust-first and the browser UI is
served locally by the Rust application.

The Platform is a separate Rust workspace. It does not import, modify, or
replace the Python Q-BenchMed core in the parent directory.

## What it does—and what it does not do

Q-BenchMed can:

- accept a folder, ZIP/TAR archive, or public GitHub repository;
- freeze the accepted bytes and retain content-addressed provenance;
- statically scan JSON, YAML, TOML, Rust, and Python without executing them;
- report parser coverage, unsupported formats, skipped checks, findings, and
  explicit limitations;
- apply installed, declarative biomedical adapters when they are eligible;
- create a reviewable `qbm.profile` containing inputs, outcomes,
  input–outcome relationships, costs, weights, constraints, and provenance;
- compare compatible approved profile versions;
- prove the smallest input panel that covers a given share of the declared
  outcomes, and the best coverage a given panel size can reach;
- run deterministic exact, certified and heuristic classical baselines;
- build QUBO and Ising models, check their logical energy equivalence, and
  report structural difficulty; and
- produce a versioned full report with audit, semantic, profile, comparison,
  optimization, quantum-formulation, and reproducibility sections.

Q-BenchMed does **not** execute uploaded source code, tests, build scripts,
models, notebooks, APIs, or neural engines. It does not derive biomedical truth
from a Python or Rust symbol name. It is not a clinical validator, diagnostic
device, regulatory assessment, general-purpose software-quality oracle, or
evidence of quantum advantage.

If the evidence cannot support a biomedical profile, Q-BenchMed records that
the profile, comparison, optimization, or QUBO/Ising check was skipped. It does
not invent the missing semantics.

Rust and Python are the currently supported source languages. JSON, YAML, and
TOML are parsed as structured data formats, not treated as programming
languages.

## Two ways to run

**Express** runs every stage unattended and hands you the report. Same stages,
same content-addressed outputs, same numbers; the approvals are stamped
`auto_accepted_by_policy` instead of with a reviewer's name, and the report says
so wherever it is shown. Use it to get numbers.

**Governed** stops at all fourteen checkpoints and asks you to approve the exact
bytes. Use it when the result has to carry a human decision.

Express is not a shortcut past safety: blocked inventory entries still stop the
run and send you to governed mode, because those entries mean the intake policy
refused something in the source.

```bash
# headless, one command, prints the full report as JSON
cargo run -p qbm-cli -- --data-dir .qbenchmed \
  run express --source /path/to/project
```

In the browser, the intake card offers the same choice; express is the default.

## Start the browser UI

From this directory:

```bash
cargo run -p qbm-cli -- --data-dir .qbenchmed serve
```

The service listens only on `127.0.0.1:8787` and opens the UI. Use `--no-open`
to print the URL without opening a browser, `--port 0` for an ephemeral port,
or install the binary and run `qbm serve` directly.

Do not open `crates/qbm-web/assets/index.html` as a `file://` page. The UI needs
the local Rust service for uploads, approvals, stored artifacts, and reports.

### Choose a source

- **Folder:** choose the project root. The browser supplies relative files;
  client absolute paths are not accepted by the API.
- **Archive:** choose a ZIP, TAR, TAR.GZ, or TGZ. Q-BenchMed scans a frozen copy
  with traversal, symlink, duplicate-path, expansion, depth, and size controls.
- **Public GitHub repository:** enter a repository-root URL such as
  `https://github.com/owner/repository` and optionally a branch or tag. The
  connector resolves it to an immutable 40-character commit before bounded
  download.

The GitHub connector currently rejects private repositories, credentials,
arbitrary Git hosts, and `/tree/...` URLs. Folder and archive analysis needs no
network access.

Use the same project display name for successive revisions if you want the
built-in heuristic profiler to give them the same stable profile identity.
An explicit profile controls its own `profile_id`.

## The 14 governed stages

Every material output is content-addressed. Approve or reject the exact output
shown at each checkpoint; a changed output invalidates an older approval.
Approval means “continue from these bytes and assumptions,” not “this project is
clinically valid.”

| Stage | Output reviewed |
| --- | --- |
| 1. Check files | Included, excluded, and blocked inventory entries |
| 2. Freeze evidence | Immutable path-to-content source snapshot |
| 3. Map structure | Scanner-neutral graph, parser diagnostics, and language inventory |
| 4. Generic source audit | Structural findings, parser coverage, skipped checks, and limitations |
| 5. Detect semantic adapters | Eligible installed data-only biomedical adapters and evidence |
| 6. Lock adapter plan | Exact adapter packages selected for this run |
| 7. Project biomedical semantics | Adapter-defined concepts and relationships |
| 8. Audit semantic model | Evidence-linked adapter policy results |
| 9. Resolve semantic views | Available and unavailable adapter projections |
| 10. Approve `qbm.profile` | Inputs, outcomes, relationships, costs, weights, constraints, and assumptions |
| 11. Compare profile versions | Semantic, reachability, coverage, panel-size, and QUBO changes |
| 12. Run classical baselines | Coverage at K, minimum panels, and solver results |
| 13. Validate QUBO and Ising | Logical models, energy-equivalence check, difficulty, and execution readiness |
| 14. Review full report | Combined report and reproducibility chain |

Stages 6–9 are marked not applicable when no installed adapter is eligible.
The profiler can still use a valid explicit `qbm.profile` or conservative
structured biomedical evidence. If neither is available, later numerical
sections remain visibly skipped. The phrase **“Generic source audit complete.”**
therefore describes the source-level checkpoint only, not a completed
biomedical optimization.

## Supplying `qbm.profile`

The most reliable path is to export `qbm.profile.json`, `qbm.profile.yaml`, or
`qbm.profile.yml`. The document must decode as `qbm.benchmark-profile/v2`, or
as `v1`, which is still accepted and means exactly what it always meant. A
document carrying either schema marker is also recognized when it has another
filename. See the
[annotated profile contract](docs/BIOMEDICAL_WORKFLOW.md#the-qbmprofile-contract)
and the [small NSCLC example](examples/profiles/nsclc-mini.qbm.profile.json).

The profile is intentionally strict:

- `profile_id` is the stable identity used across revisions;
- every input has a stable ID, positive cost, label, and sorted tags;
- every outcome has a stable ID, positive importance weight, label, and sorted
  tags;
- every relationship names one input and one outcome, and may additionally
  carry a `kind` (`required`, `supporting`, `optional`, `exclusionary`,
  `contextual`) and a `path`;
- relationships sharing an outcome and a non-empty `path` form one **arm**: all
  of them must hold together. An outcome is covered when any one of its arms
  holds, so an arm is an AND and the set of arms is an OR;
- an empty `path` means the relationship is its own single-member arm, which is
  why a `v1` document — where no relationship has a path — behaves exactly as
  it did;
- `unconditional_outcomes` names outcomes covered by every panel, including the
  empty one;
- constraints can declare selection bounds, a cost ceiling, required/excluded
  inputs, and required outcomes; and
- provenance names the exporter/revision and immutable supporting artifacts.

Inputs, outcomes, relationships, tags, constraint ID lists, and provenance
artifact IDs must be sorted and unique as documented. Dangling references,
unknown fields, invalid numbers, and inconsistent constraints are rejected.

When an explicit profile is absent, the built-in profiler considers bounded
JSON/YAML documents with conservative structured biomedical keys. It records
evidence, confidence, diagnostics, extracted fields, defaulted fields, and every
assumption. Source-language symbols are never promoted to biomedical inputs or
outcomes merely because their names look relevant.

## What the comparison and optimizers mean

The version comparison selects the latest earlier **approved** profile with the
same `profile_id`. It reports inputs, outcomes, and relationships added or
removed; newly unreachable outcomes; formerly inert inputs that became active;
coverage changes at applicable K values (5, 10, 20, 50, and 100); minimum-panel
changes at 80%, 90%, and 100% weighted coverage; and QUBO variable, coupling,
and structural-difficulty changes. A first profile has no baseline and becomes
eligible as a future baseline.

Classical analysis measures structural reachability and coverage, then runs:

- exhaustive enumeration for at most 20 inputs and at most `2^20` states;
- **certified branch-and-bound** over the native integer program, which proves
  optimality far beyond the exhaustive ceiling;
- deterministic greedy coverage-per-cost;
- seeded simulated annealing; and
- seeded tabu search.

The coverage sweep and the minimum-panel calculation both use a certified
solver: exhaustive enumeration where it is cheap, branch-and-bound beyond it.
Greedy, annealing and tabu appear in the solver comparison as baselines to
measure that certified answer against.

`optimality_proven=true` means a search closed: every alternative was either
explored or excluded by a bound that cannot discard the optimum. The certified
solver reports `false` with its remaining gap when its node budget runs out
rather than overclaiming, and heuristic results are never marked proven.

The proof covers the approved profile's relationships, costs, weights and
declared constraints. It is not a statement about anything the profile does not
contain.

### Reading the input, outcome, and K results

The completed browser report exposes the complete approved input and outcome
inventories, not only their counts. Each input is labelled **wired** when
selecting it can advance at least one outcome's rule; **veto-only** when the
rules mention it solely to block an outcome or to supply another edge's
context, so selecting it can never grant coverage; and **inert in this
qbm.profile** when no relationship mentions it at all. Each outcome is labelled
**reachable** when some panel can cover it; otherwise it is **unreachable in
this qbm.profile**, which under arm semantics can happen even to an outcome
several relationships mention, because its arms may never be satisfiable
together. These labels describe the approved profile only. They do not prove that equivalent logic is absent from unsupported,
generated, dynamic, or adapter-specific project code.

`K` is an input-panel ceiling, not a requested number of outcomes. A `K=10`
result therefore means “select at most ten inputs.” It may return fewer than ten
when every relationship that the current profile can cover is already covered.
Each K result retains the selected input IDs, covered and uncovered outcome IDs,
cost, covered weight, coverage fraction, solver, constraints, iteration count,
seed, limitations, and optimality status.

The wiring/reasoning diagnostics also state whether the adapter-specific
semantic layer ran. When no eligible declarative adapter was selected, that
layer is recorded as skipped with its reason; the approved profile may still
come from an explicit profile document or the conservative built-in profiler.

The next stage converts the approved formulation to QUBO and Ising forms,
checks their energies under bounded deterministic assignments, and reports
model size, coefficient ranges, couplings, and a structural difficulty estimate.
The default browser workflow reports quantum execution as **export-only**. It
does not contain a provider client, store provider credentials, perform hardware
embedding, submit a job, or claim quantum advantage. The provider-neutral Rust
contracts are an integration boundary for a separately reviewed executor.

## Reports, exports, and reproducibility

After the final approval, the browser offers five downloads when their
capabilities completed:

- the complete `qbm.audit-bundle/v2` report bundle;
- the approved reusable `qbm.benchmark-profile/v1` profile;
- the complete approved classical optimization analysis, including every K
  panel and its selected, covered, and uncovered IDs;
- a wiring/reasoning diagnostic that expands every input, outcome, and approved
  relationship and records the semantic-layer status; and
- the validated `qbm.quantum-formulation-export/v1` QUBO/Ising formulation.

The quantum formulation is an export for a separately configured executor, not
evidence that a provider job ran. The full report bundle contains the
`qbm.full-report/v1` report when the governed flow reaches stage 14. It retains:

- source acquisition identity and immutable source hash;
- parser coverage and unsupported-format totals;
- generic and adapter-specific findings where available;
- completed, skipped, and not-applicable capability states with reasons;
- profile evidence and assumptions;
- comparison, solver, QUBO/Ising, and difficulty outputs;
- exact stage-output hashes and approvals;
- deterministic solver seeds; and
- the tamper-evident event record used for replay checks.

Read [Biomedical workflow and architecture](docs/BIOMEDICAL_WORKFLOW.md) for
the trust boundaries, data flow, and interpretation rules. Adapter authors
should also read [Declarative adapter authoring](docs/ADAPTER_AUTHORING.md).
For a plain-language explanation of full inventories, K panels, and unwired
records, read [Reading a Q-BenchMed report](docs/READING_A_REPORT.md).

## Command line

A whole project, unattended:

```bash
qbm --data-dir .qbenchmed run express --source /path/to/project
```

A `qbm.profile` document on its own, with no run and no approval ceremony —
the same kernel and the same numbers, for a shell or for CI:

```bash
qbm bench check    qbm.profile.json   # validate and report structural facts
qbm bench analyze  qbm.profile.json   # coverage at K, minimum panels, solvers
qbm bench solve    qbm.profile.json --solver ilp --max-inputs 10
qbm bench qubo     qbm.profile.json   # QUBO/Ising sizes and difficulty
qbm bench compare  before.json after.json
```

The lower-level commands for stage-by-stage inspection and adapter development
remain available; see `qbm --help`. The browser is still the guided path for a
governed 14-stage review.

## Safety defaults

- Registered source paths are read-only and never modified.
- Browser sources are copied below the explicit data directory.
- Archives are parsed from a frozen copy and never extracted into the source
  project.
- Uploaded/imported project code is never executed.
- Python and Rust scanners perform syntax-level static extraction only.
- Declarative adapters receive no process, native-library, arbitrary-filesystem,
  network, Python, or Rust execution capability.
- Data stays below `--data-dir`, `QBM_DATA_DIR`, or `.qbenchmed`.
- The HTTP service is loopback-only and enforces Host, same-origin, CSRF,
  content-security-policy, request-body, and no-CORS boundaries.

## Build checks

Run the checks appropriate to your environment before publishing a build:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
cargo deny check
```

These commands are instructions for maintainers; this document does not claim
they passed for any particular checkout or release.

## Licence

Apache License 2.0. See [LICENSE](LICENSE).
