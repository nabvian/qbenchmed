"""Constrained-mixer QAOA on slices of the real instance, QBMED-HEME-001.

The head-to-head runs on generated instances because the penalty encoding of
the real one does not fit a simulator.  QBMED-HEME-001 is conjunctive -- most of
its rule arms need several biomarkers present together -- and expressing that as
a penalty QUBO needs an auxiliary variable per outcome plus slack.  Even a
ten-biomarker slice needs 85 qubits that way.

The constrained ansatz evaluates coverage as a diagonal phase over the inputs,
so the same slice needs ten.  This script runs it on nested slices of the real
instance and records, per slice:

  * the certified optimum (classical ILP, arm-aware) and whether greedy finds it;
  * the probability of sampling an optimal panel at depths 1-3;
  * the same probability for a uniformly random feasible panel -- the Dicke
    state before any layer -- which is the baseline QAOA has to beat, because
    the penalty baseline cannot be run at all;
  * qubit counts under both encodings, and the exact cost-layer structure
    (how many 1-, 2-, 3- and 4-body terms the AND-rules produce).

Slices keep the highest-degree biomarkers and drop any rule arm that loses a
member, so every slice is a faithful sub-problem of the real rule set.

This is a noiseless simulation.  It says nothing about hardware noise.

Writes: results/real_slices.csv
        results/real_slices_env.json
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from qbm import classical as cl
from qbm import constrained as cn
from qbm import instances as ins
from qbm import qubo as qb
from qbm import results as rs

SIZES = (10, 12, 14, 16)
DEPTHS = (1, 2, 3)
K_FRAC = 0.30
SHOTS = 4096
N_RESTARTS = 3
SEED = 0
OUT = Path(__file__).resolve().parents[1] / "results"


def main() -> None:
    t_start = time.perf_counter()
    OUT.mkdir(parents=True, exist_ok=True)
    heme = ins.heme_benchmark()
    rows: list[dict] = []

    for n in SIZES:
        s = heme.subinstance(n, instance_id=f"{heme.instance_id}-n{n}")
        K = max(2, round(K_FRAC * n))
        diag = cn.coverage_diagonal(s.A, s.w, s.arms)
        at_k, le_k = cn.optimum_at_weight(diag, K)
        if not np.isclose(at_k, le_k):
            raise AssertionError(f"n={n}: exactly-K lost the at-most-K optimum")

        certified = cl.ilp(s.A, s.w, s.c, K, arms=s.arms)
        greedy = cl.greedy(s.A, s.w, s.c, K, arms=s.arms)
        total_w = s.w.sum()
        if not np.isclose(-at_k, certified.coverage_fraction * total_w):
            raise AssertionError(f"n={n}: coverage diagonal disagrees with the ILP")

        penalty_qubits = qb.build_max_coverage(s.A, s.w, s.c, K, encoding="slack",
                                               arms=s.arms).n_vars
        # random feasible baseline: Dicke state, no layers
        p0 = cn.dicke_state(n, K).probabilities()
        random_p_opt = float(p0[diag.reshape(-1) <= at_k + 1e-9].sum())

        qa = cn.ConstrainedQaoa(s.A, s.w, K, arms=s.arms)
        terms = cn.ising_terms(diag)
        conj = sum(1 for _, m in s.arms if len(m) > 1)
        base = dict(instance_id=s.instance_id, n_inputs=n, n_outcomes=s.n_outcomes,
                    n_arms=len(s.arms), conjunctive_arms=conj, K=K,
                    certified_coverage=certified.coverage_fraction,
                    greedy_coverage=greedy.coverage_fraction,
                    greedy_finds_optimum=bool(np.isclose(greedy.coverage_fraction,
                                                         certified.coverage_fraction)),
                    penalty_qubits=penalty_qubits, constrained_qubits=n,
                    random_feasible_p_opt=random_p_opt,
                    terms_by_order=json.dumps(terms))
        for p in DEPTHS:
            out = qa.run(p, shots=SHOTS, seed=SEED, n_restarts=N_RESTARTS,
                         optimum_energy=at_k)
            r = out.resources
            rows.append({**base, "depth": p,
                         "optimum_probability": out.optimum_probability,
                         "lift_over_random": out.optimum_probability / random_p_opt,
                         "feasible_probability": out.feasible_probability,
                         "found_optimum": bool(abs(out.best_sampled_energy - at_k) < 1e-9),
                         "two_qubit_gates": r["two_qubit_gate_count"],
                         "two_qubit_gates_with_prep": r["two_qubit_gate_count_with_prep"],
                         "parameters": json.dumps(out.parameters),
                         "runtime_ms": out.runtime_ms})
        print(f"n={n} done ({time.perf_counter()-t_start:.0f}s)", flush=True)

    df = pd.DataFrame(rows)
    df.to_csv(OUT / "real_slices.csv", index=False)
    env = {"environment": rs.environment_fingerprint(),
           "benchmark_id": heme.instance_id, "instance_checksum": heme.checksum(),
           "sizes": list(SIZES), "depths": list(DEPTHS), "k_frac": K_FRAC,
           "shots": SHOTS, "n_restarts": N_RESTARTS, "seed": SEED,
           "slicing": "Instance.subinstance: highest-degree inputs, arms losing a member dropped",
           "algorithm_version": cn.ALGORITHM_VERSION,
           "total_runtime_s": round(time.perf_counter() - t_start, 1)}
    (OUT / "real_slices_env.json").write_text(json.dumps(env, indent=2))
    cols = ["n_inputs", "depth", "penalty_qubits", "constrained_qubits",
            "random_feasible_p_opt", "optimum_probability", "lift_over_random",
            "greedy_finds_optimum"]
    print(df[cols].round(4).to_string(index=False))


if __name__ == "__main__":
    main()
