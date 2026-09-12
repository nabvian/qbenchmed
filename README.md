# Q-BenchMed (Python reference implementation)

Quantum–classical benchmarking of biomedical input-selection problems.
QBMED-HEME-001: select a compact input panel maximizing weighted outcome
coverage; solved by exact ILP, greedy, simulated annealing, tabu and QAOA on
the identical QUBO.

**Status: Python reference implementation, not the Rust core.** Built to
establish mathematical correctness and locate where the quantum/classical
comparison carries information, before committing to a Rust implementation.

## Layout

    qbm/                  library
      instances.py        instance generators (6 structural regimes + heme_shaped)
      qubo.py             QUBO construction, 3 encodings, Ising conversion
      classical.py        exhaustive, ILP (HiGHS), greedy, annealing, tabu
      simulator.py        state-vector simulator + gate resource accounting
      qaoa.py             QAOA ansatz, cost/mixer, COBYLA parameter loop
      noise.py            trajectory-based noise channels
    experiments/          the four experiment scripts + report generator
    benchmarks/           exported benchmark instances (portable CSV/txt)
    results/              result tables, one CSV per experiment + env json
    reproducibility/      generated package: REPORT.md, benchmark.yaml, summary
    tests/                262 tests, incl. 40 Qiskit cross-checks
    docs/formulation.md   mathematical formulation
    figures/             publication figures

## Quick start

    pip install -e .
    pytest tests/ -q
    python experiments/run_flagship.py
    python experiments/build_report.py

See `reproducibility/README.md` for the full re-run sequence with measured
runtimes, and `reproducibility/REPORT.md` for results.

## Independent Rust Platform

The separate Rust implementation lives in [`Platform/`](Platform/README.md).
It now provides the local browser UI, folder/archive/public-GitHub intake,
static Rust/Python and structured-data scanning, declarative biomedical adapter
stages, approval-gated `qbm.profile` projection, profile-version comparison,
classical input-panel baselines, QUBO/Ising validation, and a reproducible full
report. It never imports or executes this Python reference implementation.

Project-family exporters/adapters (for sources such as PATHEX or ONCOVA) and
reviewed quantum-provider integrations remain separate extensions; the default
workflow honestly reports quantum execution as export-only. See the Platform
[roadmap](Platform/ROADMAP.md) and the retained original
[blueprint](docs/platform/README.md).

## Scope

The 66x88 instance is a SYNTHETIC STAND-IN; the pathology knowledge export
described in the project specification was never available in this workspace.
No clinical claims, no quantum advantage claims. See REPORT.md sections
"Scope and what this is not" and 9 (Limitations).
