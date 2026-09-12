"""Structural survey: where do methods separate at all?

Greedy carries a 1-1/e guarantee on submodular coverage and in practice does
much better, so most instances of this problem are ties.  This experiment finds
the cells of the design space where greedy FAILS to reach the certified
optimum -- those are the only cells where a comparison against QAOA can carry
information.  The head-to-head then runs in the hardest cell that also fits in
a state-vector simulator.

Grid: 6 structural regimes x 7 sizes x 3 budget fractions x 5 seeds, plus the
real flagship profile at the same budget fractions as a reference row.  The
flagship is included because a survey of synthetic structure is only useful if
it brackets the instance the project actually cares about.

Writes: results/discriminating_records.jsonl  (canonical)
        results/discriminating_power.csv      (derived view for the report)
        results/discriminating_power_env.json
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

RES = Path(__file__).resolve().parents[1] / "results"

REGIMES = ("low_overlap", "high_overlap", "clustered", "sparse", "dense", "degree_capped")
SIZES = (10, 16, 22, 30, 40, 50, 66)   # spec section 18 scaling ladder
K_FRACS = (0.15, 0.25, 0.40)
SEEDS = (0, 1, 2, 3, 4)


def cells():
    """Every (instance, K) cell in the survey, synthetic grid then flagship."""
    for regime, n_in, k_frac, seed in itertools.product(REGIMES, SIZES, K_FRACS, SEEDS):
        # Outcome count tracks the flagship's 66:88 ratio at every size, so the
        # survey scales the real shape rather than an arbitrary one.
        n_out = int(round(n_in * 88 / 66))
        inst = ins.generate(regime, n_in, n_out, seed=seed)
        yield inst, k_frac, seed, "QBM-SYN"
    flagship = ins.heme_benchmark()
    for k_frac in K_FRACS:
        yield flagship, k_frac, 0, "QBMED-HEME-001"


def main() -> None:
    t_start = time.perf_counter()
    RES.mkdir(exist_ok=True)

    allrecs = rs.ResultSet()
    rows = []
    for inst, k_frac, seed, bid in cells():
        K = max(1, int(round(k_frac * inst.n_inputs)))
        spec = rn.RunSpec(benchmark_id=bid, input_budget=K,
                          notes="structural discriminating-power survey")
        out = rn.run_comparison(inst, spec, [{"algorithm": "ilp"},
                                             {"algorithm": "greedy"}])
        by = {r.algorithm: r for r in out}
        opt, greedy = by["ilp"], by["greedy"]
        for r in out:
            allrecs.append(r)
        rows.append({
            "regime": inst.regime, "n_in": inst.n_inputs, "K": K,
            "K_frac": k_frac, "seed": seed, "benchmark_id": bid,
            "opt": opt.objective_value, "greedy": greedy.objective_value,
            "certified": opt.certified_optimal,
            "ilp_ms": opt.runtime_ms, "greedy_ms": greedy.runtime_ms,
            "opt_cov": opt.coverage_fraction,
            "greedy_cov": greedy.coverage_fraction,
            "is_conjunctive": inst.is_conjunctive,
            "greedy_bound_applies": bool(
                greedy.extra.get("submodular_bound_applies", False)),
            # RELATIVE gap against the certified optimum, taken from the
            # record's attached reference.  Relative rather than absolute so it
            # is comparable across sizes, where the objective magnitude grows
            # with the number of outcomes.
            "gap": abs(greedy.optimality_gap_rel or 0.0),
            "greedy_optimal": bool(abs(greedy.optimality_gap_abs) < 1e-9),
        })

    allrecs.to_jsonl(RES / "discriminating_records.jsonl")
    df = pd.DataFrame(rows)
    df.to_csv(RES / "discriminating_power.csv", index=False)

    env = {
        "environment": rs.environment_fingerprint(),
        "regimes": list(REGIMES), "sizes": list(SIZES),
        "k_fracs": list(K_FRACS), "seeds": list(SEEDS),
        "flagship_included": True,
        "schema_version": rs.SCHEMA_VERSION,
        "total_runtime_s": round(time.perf_counter() - t_start, 1),
    }
    (RES / "discriminating_power_env.json").write_text(json.dumps(env, indent=2))
    print(f"{len(df)} cells, {len(allrecs)} records -> {RES}")
    print(df.groupby("regime").greedy_optimal.mean().round(3).to_string())


if __name__ == "__main__":
    main()
