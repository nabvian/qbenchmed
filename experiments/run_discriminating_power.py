"""Structural survey: where do methods separate at all?

Greedy carries a 1-1/e guarantee on submodular coverage and in practice does
much better, so most instances of this problem are ties. This experiment finds
the cells of the design space where greedy FAILS to reach the certified
optimum -- those are the only cells where a comparison against QAOA can carry
information. The head-to-head then runs in the hardest cell that also fits in
a state-vector simulator.

Grid: 6 structural regimes x 7 sizes x 3 budget fractions x 5 seeds.

Writes: results/discriminating_power.csv, results/discriminating_power_env.json
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
from qbm import classical as qcl
from qbm import instances as ins

RES = Path(__file__).resolve().parents[1] / "results"

REGIMES = ("low_overlap", "high_overlap", "clustered", "sparse", "dense", "degree_capped")
SIZES = (10, 16, 22, 30, 40, 50, 66)   # spec section 18 scaling ladder
K_FRACS = (0.15, 0.25, 0.40)
SEEDS = (0, 1, 2, 3, 4)


def main() -> None:
    t_start = time.perf_counter()
    RES.mkdir(exist_ok=True)

    rows = []
    for regime, n_in, k_frac, seed in itertools.product(REGIMES, SIZES, K_FRACS, SEEDS):
        # Outcome count tracks the flagship's 66:88 ratio at every size, so
        # the survey scales the real shape rather than an arbitrary one.
        n_out = int(round(n_in * 88 / 66))
        inst = ins.generate(regime, n_in, n_out, seed=seed)
        K = max(1, int(round(k_frac * n_in)))
        A, w, c = inst.A, inst.w, inst.c

        r_opt = qcl.ilp(A, w, c, K)
        r_greedy = qcl.greedy(A, w, c, K)
        rows.append({
            "regime": regime, "n_in": n_in, "K": K, "K_frac": k_frac, "seed": seed,
            "opt": r_opt.objective_value, "greedy": r_greedy.objective_value,
            "certified": bool(r_opt.extra.get('certified_optimal', False)),
            "ilp_ms": r_opt.runtime_ms, "greedy_ms": r_greedy.runtime_ms,
            "opt_cov": r_opt.coverage_fraction, "greedy_cov": r_greedy.coverage_fraction,
            # RELATIVE gap against the certified optimum. Relative rather than
            # absolute so it is comparable across sizes, where the objective
            # magnitude grows with the number of outcomes.
            "gap": (0.0 if abs(r_opt.objective_value) < 1e-12 else
                    abs(r_greedy.objective_value - r_opt.objective_value)
                    / abs(r_opt.objective_value)),
            "greedy_optimal": bool(abs(r_greedy.objective_value - r_opt.objective_value) < 1e-9),
        })

    df = pd.DataFrame(rows)
    df.to_csv(RES / "discriminating_power.csv", index=False)

    env = {
        "python": platform.python_version(),
        "platform": platform.platform(),
        "numpy": np.__version__,
        "regimes": list(REGIMES), "sizes": list(SIZES),
        "k_fracs": list(K_FRACS), "seeds": list(SEEDS),
        "total_runtime_s": round(time.perf_counter() - t_start, 1),
    }
    (RES / "discriminating_power_env.json").write_text(json.dumps(env, indent=2))
    print(f"{len(df)} instances -> {RES / 'discriminating_power.csv'}")
    print(df.groupby("regime").greedy_optimal.mean().round(3).to_string())


if __name__ == "__main__":
    main()
