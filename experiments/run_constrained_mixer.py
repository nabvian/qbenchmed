"""Constrained mixer against the penalty encoding, on the head-to-head instances.

The head-to-head found that QAOA at its best depth is comparable to the
classical heuristics, and the penalty-landscape measurement said why: the budget
penalty runs 85-374x the coverage signal and leaves under 1% of the spectrum
useful.  That measurement makes a prediction.  Remove the penalty -- never
leave the feasible set -- and the probability of sampling the optimum should
rise.

This experiment tests the prediction.  On the SAME generated instances the
head-to-head uses (same regime, sizes, seeds, budget fraction, shots and
restart count), it runs two ansatzes side by side:

  penalty      standard X mixer from |+>^n, budget as a penalty, slack qubits
  constrained  XY ring mixer from the Dicke state |D^n_K>, no penalty, no slack

Both are scored against the same certified optimum, and both report the two
things that matter: how much probability lands on the optimum, and what the
circuit costs in two-qubit gates on CX-native hardware.  The Dicke preparation
is priced separately, from the CNOT count of the real preparation circuit (see
qbm.constrained.dicke_prep_cost), because it is a one-time cost the simulator's
direct construction would otherwise hide.

This is a proof of concept on a noiseless simulator.  It says nothing about
hardware noise.

Writes: results/constrained_mixer.csv
        results/constrained_mixer_env.json
"""
from __future__ import annotations

import itertools
import json
from math import comb
import sys
import time
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from qbm import constrained as cn
from qbm import instances as ins
from qbm import qubo as qb
from qbm import results as rs
from qbm.qaoa import Qaoa, ising_resource_cost

# Identical to run_headtohead.py so the two experiments share instances.
REGIME = "degree_capped"
SIZES = (8, 10, 12, 14, 16)
DEPTHS = (1, 2, 3)
SEEDS = (0, 1, 2, 3, 4)
K_FRAC = 0.30
SHOTS = 4096
N_RESTARTS = 3
OUT = Path(__file__).resolve().parents[1] / "results"


def main() -> None:
    t_start = time.perf_counter()
    OUT.mkdir(parents=True, exist_ok=True)
    rows: list[dict] = []

    for n_in, seed in itertools.product(SIZES, SEEDS):
        n_out = int(round(n_in * 88 / 66))
        K = max(2, round(K_FRAC * n_in))
        inst = ins.generate(REGIME, n_in, n_out, seed=seed)

        # Ground truth: the certified optimum over "at most K".  The exactly-K
        # subspace must not lose it; assert rather than assume.
        diag = cn.coverage_diagonal(inst.A, inst.w)
        at_k, le_k = cn.optimum_at_weight(diag, K)
        if not np.isclose(at_k, le_k):
            raise AssertionError(
                f"n={n_in} seed={seed}: exactly-K optimum {at_k} != at-most-K "
                f"{le_k}; the subspace restriction would lose the optimum here")

        q = qb.build_max_coverage(inst.A, inst.w, inst.c, K,
                                  encoding="pairwise", arms=inst.arms)
        _, E = q.all_energies()
        qubo_opt = float(E.min())
        pen = Qaoa(q)
        con = cn.ConstrainedQaoa(inst.A, inst.w, K)

        # The fair baseline: a uniformly random panel of exactly K biomarkers,
        # i.e. the Dicke state before any layer.  Without it a 100% "hit" rate
        # means nothing -- 4,096 shots over a feasible space of at most 4,368
        # panels finds the optimum by chance alone.
        p0 = cn.dicke_state(n_in, K).probabilities()
        random_p_opt = float(p0[diag.reshape(-1) <= at_k + 1e-9].sum())

        base = dict(regime=REGIME, n_inputs=n_in, n_outcomes=inst.n_outcomes,
                    K=K, instance_seed=seed, coverage_optimum=-at_k,
                    feasible_space=int(comb(n_in, K)),
                    random_feasible_p_opt=random_p_opt,
                    random_feasible_hit=1.0 - (1.0 - random_p_opt) ** SHOTS)

        for p in DEPTHS:
            po = pen.run(p, shots=SHOTS, seed=seed, n_restarts=N_RESTARTS,
                         optimum_energy=qubo_opt,
                         feasible_fn=lambda x, n=n_in, k=K: x[:n].sum() <= k)
            pres = ising_resource_cost(pen.h, pen.J, p)
            rows.append({**base, "ansatz": "penalty", "depth": p,
                         "qubits": q.n_vars,
                         "optimum_probability": po.optimum_probability,
                         "lift_over_random": po.optimum_probability / random_p_opt,
                         "feasible_probability": po.feasible_probability,
                         "found_optimum": bool(po.best_feasible_energy is not None
                                               and abs(po.best_feasible_energy - qubo_opt) < 1e-9),
                         "two_qubit_gates": pres["two_qubit_gate_count"],
                         "two_qubit_gates_with_prep": pres["two_qubit_gate_count"],
                         "runtime_ms": po.runtime_ms})

            co = con.run(p, shots=SHOTS, seed=seed, n_restarts=N_RESTARTS,
                         optimum_energy=at_k)
            cr = co.resources
            rows.append({**base, "ansatz": "constrained", "depth": p,
                         "qubits": co.n_qubits,
                         "optimum_probability": co.optimum_probability,
                         "lift_over_random": co.optimum_probability / random_p_opt,
                         "feasible_probability": co.feasible_probability,
                         "found_optimum": bool(abs(co.best_sampled_energy - at_k) < 1e-9),
                         "two_qubit_gates": cr["two_qubit_gate_count"],
                         "two_qubit_gates_with_prep": cr["two_qubit_gate_count_with_prep"],
                         "runtime_ms": co.runtime_ms})
        print(f"n_in={n_in} seed={seed} done ({time.perf_counter()-t_start:.0f}s)", flush=True)

    df = pd.DataFrame(rows)
    df.to_csv(OUT / "constrained_mixer.csv", index=False)
    env = {"environment": rs.environment_fingerprint(),
           "regime": REGIME, "sizes": list(SIZES), "depths": list(DEPTHS),
           "seeds": list(SEEDS), "shots": SHOTS, "n_restarts": N_RESTARTS,
           "k_frac": K_FRAC,
           "instances_shared_with": "experiments/run_headtohead.py",
           "cnot_per_rzz": cn.CNOT_PER_RZZ, "cnot_per_xy": cn.CNOT_PER_XY,
           "dicke_prep_basis": "CNOTs in the emitted Baertschi-Eidenbenz circuit, untranspiled",
           "algorithm_version": cn.ALGORITHM_VERSION,
           "total_runtime_s": round(time.perf_counter() - t_start, 1)}
    (OUT / "constrained_mixer_env.json").write_text(json.dumps(env, indent=2))
    print(f"\n{len(df)} rows -> {OUT / 'constrained_mixer.csv'}")
    print(df.groupby(["ansatz", "depth"]).optimum_probability.mean().round(4).to_string())


if __name__ == "__main__":
    main()
