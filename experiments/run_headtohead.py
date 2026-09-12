"""Head-to-head: every solver on the identical QUBO object.

This is the experiment the whole framework exists to make fair (spec section
50).  Exhaustive enumeration, simulated annealing, tabu and QAOA at three
depths all minimise *the same* Qubo instance -- not a re-derivation of it --
so a difference in objective is a difference in optimizer, not in problem.

Greedy is included but tagged formulation="native": it operates on the coverage
structure, not on a QUBO.  It is placed on the QUBO axis by holding its
selection fixed and completing the auxiliary variables optimally, which changes
nothing about what greedy does and makes its answer commensurable.  It is never
averaged with the QUBO solvers.

Regime and size come from the discriminating-power survey: the cell chosen is
the one where greedy most often fails to reach the certified optimum, among the
sizes a state-vector simulator can hold.  Running the comparison where every
method ties would measure nothing.

Wall-clock across the quantum/classical boundary is NOT comparable -- a
state-vector simulation of QAOA is not a QPU -- and every quantum record is
flagged accordingly.  Runtimes are reported within a family only.

Writes: results/headtohead_records.jsonl  (canonical)
        results/headtohead.csv            (derived view for the report)
        results/headtohead_env.json
"""
from __future__ import annotations

import itertools
import json
import sys
import time
from pathlib import Path

import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from qbm import instances as ins
from qbm import results as rs
from qbm import runner as rn

REGIME = "degree_capped"     # pairwise encoding is exact only at outcome degree <= 2
SIZES = (8, 10, 12, 14, 16)
DEPTHS = (1, 2, 3)
SEEDS = (0, 1, 2, 3, 4)
K_FRAC = 0.30
ENCODING = "pairwise"
SHOTS = 4096
N_RESTARTS = 3
OUT = Path(__file__).resolve().parents[1] / "results"


def main() -> None:
    t_start = time.perf_counter()
    OUT.mkdir(parents=True, exist_ok=True)

    allrecs = rs.ResultSet()
    rows: list[dict] = []

    for n_in, seed in itertools.product(SIZES, SEEDS):
        n_out = int(round(n_in * 88 / 66))
        K = max(2, round(K_FRAC * n_in))
        inst = ins.generate(REGIME, n_in, n_out, seed=seed)
        spec = rn.RunSpec(benchmark_id="QBM-H2H", input_budget=K,
                          encoding=ENCODING, notes="head-to-head on one QUBO")

        # qubo_exhaustive runs first (the runner orders exact algorithms first),
        # so its ground-state energy becomes the QUBO-axis reference every other
        # record is scored against, QAOA included.
        plan = [{"algorithm": "qubo_exhaustive"},
                {"algorithm": "greedy"},
                {"algorithm": "simulated_annealing", "seed": seed},
                {"algorithm": "tabu", "seed": seed}]
        plan += [{"algorithm": "qaoa", "depth": p, "shots": SHOTS, "seed": seed,
                  "n_restarts": N_RESTARTS} for p in DEPTHS]
        out = rn.run_comparison(inst, spec, plan)
        for r in out:
            allrecs.append(r)

        gt = next(r for r in out if r.algorithm == "qubo_exhaustive")
        base = dict(regime=REGIME, n_inputs=n_in, n_outcomes=inst.n_outcomes,
                    K=K, encoding=ENCODING, qubits=gt.n_problem_variables,
                    instance_seed=seed, optimum=gt.qubo_energy,
                    n_optimal_states=gt.extra.get("n_optimal_states"),
                    optimum_density=gt.extra.get("optimum_density"))

        for r in out:
            name = (f"qaoa_p{r.qaoa_depth}" if r.algorithm == "qaoa"
                    else "annealing" if r.algorithm == "simulated_annealing"
                    else "exhaustive" if r.algorithm == "qubo_exhaustive"
                    else r.algorithm)
            sm = r.sampling_metrics or {}
            qres = (r.quantum_resources or {}).get("resources", {})
            rows.append({
                **base, "algorithm": name,
                "family": "quantum" if r.algorithm == "qaoa" else "classical",
                "formulation": r.formulation, "depth": r.qaoa_depth,
                # The comparison axis is QUBO energy: the one quantity every
                # method above produces for the same object.
                "objective": r.qubo_energy,
                "gap": r.qubo_gap_abs,
                "found_optimum": bool(abs(r.qubo_gap_abs) < 1e-9),
                "runtime_ms": r.runtime_ms,
                "wall_clock_comparable": r.wall_clock_comparable,
                "shots": r.shots,
                "two_qubit_gates": qres.get("two_qubit_gate_count"),
                "optimum_probability": sm.get("optimum_probability"),
                "near_optimum_probability": sm.get("near_optimum_probability"),
                "expectation": sm.get("final_expectation"),
                "feasible_probability": sm.get("feasible_probability"),
                # Native-axis coverage, so the QUBO winner can be checked
                # against what it actually selects.
                "coverage_fraction": r.coverage_fraction,
                "n_selected": r.n_selected,
            })
        print(f"n_in={n_in} seed={seed} done ({time.perf_counter()-t_start:.0f}s)")

    allrecs.to_jsonl(OUT / "headtohead_records.jsonl")
    df = pd.DataFrame(rows)
    df.to_csv(OUT / "headtohead.csv", index=False)

    env = {"environment": rs.environment_fingerprint(),
           "regime": REGIME, "encoding": ENCODING, "sizes": list(SIZES),
           "depths": list(DEPTHS), "seeds": list(SEEDS), "shots": SHOTS,
           "n_restarts": N_RESTARTS, "k_frac": K_FRAC,
           "schema_version": rs.SCHEMA_VERSION,
           "regime_chosen_from": "results/discriminating_power.csv",
           "total_runtime_s": round(time.perf_counter() - t_start, 1)}
    (OUT / "headtohead_env.json").write_text(json.dumps(env, indent=2))
    print(f"\n{len(df)} rows, {len(allrecs)} records -> {OUT}")
    print(df.groupby("algorithm").found_optimum.mean().round(3).to_string())


if __name__ == "__main__":
    main()
