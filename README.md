# Q-BenchMed

Quantum–classical benchmarking of biomedical input-selection problems: select a
compact input panel maximizing weighted outcome coverage, solved by exact ILP,
greedy, simulated annealing, tabu and QAOA on the identical QUBO.

The repository holds two implementations that share a problem definition and
nothing else:

| | `qbm/` (Python) | `Platform/` (Rust) |
|---|---|---|
| Role | reference implementation; establishes that the mathematics is correct | the product: intake, profiling, optimization, reporting, browser UI |
| Instance | `QBMED-HEME-001`, the real 66×88 rule graph | any uploaded project, profiled on the spot |
| Quantum | QAOA on a state-vector simulator, with noise | QUBO/Ising formulation and export |
| Entry point | `experiments/*.py` | `qbm serve` / `qbm run express` |

The Python core does not import the Rust Platform, and the Platform never
executes the Python core.

## Python reference implementation

    qbm/                  library
      instances.py        instance generators (6 structural regimes + heme_shaped)
      profile.py          domain-profile layer: typed edges and path groups
      domains/heme.py     the 66-input / 88-outcome hematology adapter
      qubo.py             QUBO construction, 3 encodings, Ising conversion
      classical.py        exhaustive, ILP (HiGHS), greedy, annealing, tabu
      runner.py           one code path that prepares every solver's problem
      results.py          one record format across classical and quantum runs
      simulator.py        state-vector simulator + gate resource accounting
      qaoa.py             QAOA ansatz, cost/mixer, COBYLA parameter loop
      noise.py            trajectory-based noise channels
    experiments/          the four experiment scripts + report generator
    benchmarks/heme/      the exported real profile
    results/              result tables, one CSV per experiment + env json
    reproducibility/      generated package: REPORT.md, benchmark.yaml, summary
    tests/                354 tests, incl. 40 Qiskit cross-checks
    docs/formulation.md   mathematical formulation
    figures/              publication figures

Python >= 3.11. Runtime deps are numpy, scipy, pandas. Qiskit is a *validation
reference only* (40 tests cross-check the simulator against it) and lives in the
`dev` extra; the library never imports it.

    python3 -m venv .venv
    source .venv/bin/activate          # Windows: .venv\Scripts\activate
    pip install -e ".[dev]"

    pytest tests/ -q                   # 354 tests, ~20 s

    python experiments/run_flagship.py              #  1 s
    python experiments/run_discriminating_power.py  # 10 s
    python experiments/build_report.py              # regenerates reproducibility/

The two long experiments are optional — their result CSVs ship in `results/`,
so `build_report.py` works without re-running them:

    python experiments/run_headtohead.py   #  8 min  (state-vector simulation)
    python experiments/run_noise.py        # 26 min  (315 points x 1500 trajectories)

See `reproducibility/README.md` for the full re-run sequence with measured
runtimes, and `reproducibility/REPORT.md` for results.

## Rust Platform

The Platform lives in [`Platform/`](Platform/README.md) and is the part you run
on your own projects. It accepts a folder, archive, or public GitHub repository,
derives a reviewable `qbm.profile`, and runs classical baselines and the
QUBO/Ising formulation against it.

    cd Platform
    cargo run -p qbm-cli -- --data-dir .qbenchmed run express --source /path/to/project

or `cargo run -p qbm-cli -- serve` for the browser UI.

Two run modes:

- **express** — the whole pipeline runs unattended and every stage is stamped
  `auto_accepted_by_policy`. Use it to get numbers.
- **governed** — you approve the exact bytes at every one of the fourteen
  checkpoints. Use it when the result has to carry a human decision.

The Platform's own README documents the stages, the profile contract, the safety
boundaries, and what it deliberately does not do.

## Scope

`QBMED-HEME-001` is built from an audit export of a pathology rule engine's
outcome-by-input trigger and wiring matrix (audit snapshot 2026-08-29, PRO-EXEC
bundle 1.2.0). It records which biomarkers each rule reads. It contains
no patient data, and nothing here is a statement about hematology: a covering
panel is a solution to a coverage objective over a rule graph, and it does not
follow that any test is clinically unnecessary or that a smaller panel is safe.

No clinical claims. No quantum advantage claims. See `reproducibility/REPORT.md`
sections "Scope and what this is not" and 9 (Limitations).
