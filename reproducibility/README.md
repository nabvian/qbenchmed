# QBMED-HEME-001 — Reproducibility Package

Everything needed to re-run the benchmark and regenerate this package.

## Contents

| file | what it is |
|---|---|
| `benchmark.yaml` | The benchmark definition: instance, objective, encodings, algorithms, seeds, and the claims policy. |
| `REPORT.md` | Generated report. Every number is rendered from the result CSVs by `report_template.render()` — none are typed in. |
| `results.json` | Machine-readable summary statistics, the same dict the report renders from. |
| `summary.csv` | Per-algorithm summary rows: success rate, absolute gap, runtime, two-qubit gates. |
| `environment.json` | Python, platform and library versions at generation time. |
| `parameters.json` | Parameters as recorded *by the experiment scripts themselves*, not restated. |

## Re-running

Runtimes below are the measured wall-clock times recorded by each script in its
own `results/*_env.json`, on the platform in `environment.json`.

```bash
# 1. flagship 66x88 coverage curve, K = 1..66        (1 s)
python experiments/run_flagship.py

# 2. structural survey, 630 instances               (10 s)
python experiments/run_discriminating_power.py

# 3. head-to-head QAOA vs classical, 50 instances   (8 min)
python experiments/run_headtohead.py

# 4. noise sweep, 315 points x 1500 trajectories    (26 min)
python experiments/run_noise.py

# 5. regenerate this package
python experiments/build_report.py
```

Steps 1-2 are fast because they are pure classical optimization; steps 3-4 are
dominated by state-vector simulation. That asymmetry is itself the headline
result — see REPORT.md section 1.

Then `pytest tests/ -q` for the suite (262 tests, including cross-validation of
the state-vector simulator against Qiskit and the paired ideal/noisy path check).

## Determinism

Instance generation, simulated annealing, tabu search, QAOA parameter
initialisation and noise trajectories are all seeded. The recorded seeds are in
`parameters.json`. Re-running with the same seeds on the same platform
reproduces the CSVs; across platforms, expect BLAS-level floating-point
differences in the last digits and identical solver decisions.

## Two things to know before reading the results

**The instance is a synthetic stand-in.** The 66×88 pathology knowledge export
described in the specification was not available. `heme_shaped` generates an
instance matched to its declared shape. It has no clinical content.

**One experiment was discarded.** `results/noise_sweep_INVALID_phasebug.csv` is
the first noise sweep, invalidated by a branch-cut bug in the noisy cost phase.
It is kept rather than deleted so the discard is auditable. Do not use it.
REPORT.md section 7 explains the bug.
