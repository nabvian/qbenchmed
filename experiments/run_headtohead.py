"""Head-to-head: QAOA vs classical baselines on identical QUBOs (spec1 s16.4, s50).

Fairness rules enforced here, because they are what makes the comparison mean
anything:

* Every algorithm receives the *same* QUBO object -- same instance, same
  encoding, same penalty coefficients.  Nothing is re-derived per solver.
* Ground truth comes from exhaustive enumeration of that same QUBO, so the
  optimality gap is measured against the true minimum of the problem actually
  being solved, not of a related one.
* Stochastic methods get multiple independent seeds; a single run would conflate
  algorithm quality with luck.
* Wall-clock time is recorded but never used to rank quantum against classical:
  a state-vector simulation of QAOA on a CPU is not a QPU execution, and spec1
  s50 forbids conflating the two.  Runtime is reported for the classical
  methods against each other, and for QAOA only as simulation cost.

Run: python experiments/run_headtohead.py
"""

from __future__ import annotations

import itertools
import json
import platform
import sys
import time
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from qbm import classical as qc          # noqa: E402
from qbm import instances as ins         # noqa: E402
from qbm import qaoa as qa               # noqa: E402
from qbm import qubo as qb               # noqa: E402

SIZES = (8, 10, 12, 14, 16)
DEPTHS = (1, 2, 3)
SEEDS = tuple(range(10))
REGIME = "degree_capped"          # the only regime that both discriminates and fits
K_FRAC = 0.4
ENCODING = "pairwise"             # exact for outcome degree <= 2
SHOTS = 4096
N_RESTARTS = 3
OUT = Path(__file__).resolve().parents[1] / "results"


def build(n_in: int, seed: int):
    n_out = int(round(n_in * 88 / 66))
    K = max(2, round(K_FRAC * n_in))
    inst = ins.generate(REGIME, n_in, n_out, seed=seed)
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, K=K, encoding=ENCODING)
    return inst, q, K


def main() -> None:
    rows: list[dict] = []
    t_start = time.perf_counter()

    for n_in, seed in itertools.product(SIZES, SEEDS):
        inst, q, K = build(n_in, seed)
        # ground truth on the identical QUBO
        Z, E = q.all_energies()
        opt = float(E.min())
        n_opt = int(np.sum(E <= opt + 1e-9))
        base = dict(regime=REGIME, n_inputs=n_in, n_outcomes=inst.n_outcomes,
                    K=K, encoding=ENCODING, qubits=q.n_vars, instance_seed=seed,
                    optimum=opt, n_optimal_states=n_opt,
                    optimum_density=n_opt / len(E))

        # ---- classical QUBO solvers: the identical Qubo object ----------
        # simulated_annealing and tabu take a Qubo directly, so they minimise
        # exactly the function QAOA minimises -- no re-derivation anywhere.
        for name, fn in (("annealing", qc.simulated_annealing), ("tabu", qc.tabu)):
            t0 = time.perf_counter()
            z, energy, meta = fn(q, seed=seed)
            rows.append({**base, "algorithm": name, "family": "classical",
                         "formulation": "qubo", "depth": None,
                         "objective": float(energy),
                         "gap": float(energy) - opt,
                         "found_optimum": abs(float(energy) - opt) < 1e-9,
                         "runtime_ms": (time.perf_counter() - t0) * 1e3,
                         "shots": None, "two_qubit_gates": None,
                         "optimum_probability": None})

        # exhaustive enumeration of the same QUBO: this is the ground truth, so
        # it is recorded as a row too rather than being implicit
        rows.append({**base, "algorithm": "exhaustive", "family": "classical",
                     "formulation": "qubo", "depth": None, "objective": opt,
                     "gap": 0.0, "found_optimum": True,
                     "runtime_ms": float("nan"),   # measured once below, not per row
                     "shots": None, "two_qubit_gates": None,
                     "optimum_probability": None})

        # ---- greedy: native formulation, reported separately ------------
        # Greedy operates on the coverage structure, not on a QUBO, so its
        # objective lives in a different space.  To place it on the QUBO axis
        # without changing what greedy does, its selection is held fixed and the
        # QUBO is minimised over the remaining (auxiliary) variables exactly,
        # using the enumeration already computed.  The row stays tagged
        # formulation="native" so it is never averaged with the QUBO solvers.
        g = qc.greedy(inst.A, inst.w, inst.c, K)
        x_g = np.zeros(n_in, bool)
        x_g[list(g.selected_inputs)] = True
        xs = np.array([q.varmap.x_of(z) for z in Z])
        match = np.all(xs == x_g, axis=1)
        g_energy = float(E[match].min()) if match.any() else float("nan")
        rows.append({**base, "algorithm": "greedy", "family": "classical",
                     "formulation": "native", "depth": None,
                     "objective": g_energy, "gap": g_energy - opt,
                     "found_optimum": abs(g_energy - opt) < 1e-9,
                     "runtime_ms": g.runtime_ms, "shots": None,
                     "two_qubit_gates": None, "optimum_probability": None})

        # ---- QAOA at each depth on the same QUBO ------------------------
        for p in DEPTHS:
            out = qa.Qaoa(q).run(p, shots=SHOTS, seed=seed,
                                 n_restarts=N_RESTARTS, optimum_energy=opt)
            rows.append({**base, "algorithm": f"qaoa_p{p}", "family": "quantum",
                         "formulation": "qubo",
                         "depth": p, "objective": out.best_sampled_energy,
                         "gap": out.best_sampled_energy - opt,
                         "found_optimum": abs(out.best_sampled_energy - opt) < 1e-9,
                         "runtime_ms": out.runtime_ms, "shots": SHOTS,
                         "two_qubit_gates": out.resources["two_qubit_gate_count"],
                         "optimum_probability": out.optimum_probability,
                         "expectation": out.final_expectation,
                         "near_optimum_probability": out.near_optimum_probability,
                         "optimizer_evals": out.optimizer.function_evaluations})
        print(f"n_in={n_in} seed={seed} done ({time.perf_counter()-t_start:.0f}s)")

    df = pd.DataFrame(rows)
    OUT.mkdir(parents=True, exist_ok=True)
    df.to_csv(OUT / "headtohead.csv", index=False)

    env = {"python": sys.version.split()[0], "platform": platform.platform(),
           "numpy": np.__version__, "regime": REGIME, "encoding": ENCODING,
           "sizes": list(SIZES), "depths": list(DEPTHS), "seeds": list(SEEDS),
           "shots": SHOTS, "n_restarts": N_RESTARTS, "k_frac": K_FRAC,
           "total_runtime_s": round(time.perf_counter() - t_start, 1)}
    (OUT / "headtohead_env.json").write_text(json.dumps(env, indent=2))
    print(f"\n{len(df)} rows -> {OUT/'headtohead.csv'}")


if __name__ == "__main__":
    main()
