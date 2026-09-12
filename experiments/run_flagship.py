"""Flagship 66x88 benchmark: full coverage curve with certified optima.

Sweeps the input budget K from 1 to 66. At each K, exact ILP (HiGHS) gives a
certified optimum and greedy gives the submodular baseline. The point of the
experiment is to establish whether the flagship instance is hard enough to be
a candidate for any speedup -- so ILP runtime is recorded per point.

The instance is a SYNTHETIC STAND-IN for the pathology knowledge export
described in the specification; see reproducibility/benchmark.yaml.

Writes: results/flagship_coverage_curve.csv, results/flagship_env.json
"""
from __future__ import annotations

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
SEED = 42


def main() -> None:
    t_start = time.perf_counter()
    RES.mkdir(exist_ok=True)

    inst = ins.heme_benchmark(seed=SEED)
    A, w, c = inst.A, inst.w, inst.c
    n_in, n_out = A.shape

    rows = []
    for K in range(1, n_in + 1):
        r_ilp = qcl.ilp(A, w, c, K)
        r_greedy = qcl.greedy(A, w, c, K)
        rows.append({
            "K": K,
            "ilp_cov": r_ilp.coverage_fraction,
            "greedy_cov": r_greedy.coverage_fraction,
            "ilp_obj": r_ilp.objective_value,
            "greedy_obj": r_greedy.objective_value,
            "certified": bool(r_ilp.extra.get('certified_optimal', False)),
            "ilp_ms": r_ilp.runtime_ms,
            "n_sel": len(r_ilp.selected_inputs),
            # Greedy optimality is judged against the CERTIFIED optimum, not
            # against a heuristic reference.
            "greedy_is_optimal": bool(
                abs(r_greedy.objective_value - r_ilp.objective_value) < 1e-9),
            "rel_gap": (0.0 if abs(r_ilp.objective_value) < 1e-12 else
                        abs(r_greedy.objective_value - r_ilp.objective_value) / abs(r_ilp.objective_value)),
        })

    df = pd.DataFrame(rows)
    df.to_csv(RES / "flagship_coverage_curve.csv", index=False)

    env = {
        "python": platform.python_version(),
        "platform": platform.platform(),
        "numpy": np.__version__,
        "n_inputs": int(n_in),
        "n_outcomes": int(n_out),
        "generator": "heme_shaped",
        "seed": SEED,
        "provenance": "synthetic stand-in for the 66x88 pathology export",
        "total_runtime_s": round(time.perf_counter() - t_start, 1),
    }
    (RES / "flagship_env.json").write_text(json.dumps(env, indent=2))
    print(f"{len(df)} budgets -> {RES / 'flagship_coverage_curve.csv'}")
    print(f"all certified: {bool(df.certified.all())}; "
          f"min K for full coverage: {int(df[df.ilp_cov >= 1 - 1e-9].K.min())}")


if __name__ == "__main__":
    main()
