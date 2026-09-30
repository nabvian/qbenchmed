"""Flagship 66x88 benchmark: full coverage curve with certified optima.

Sweeps the input budget K from 1 to 66 on the real Q-BenchMed-Heme profile.
At each K, exact ILP (HiGHS) gives a certified optimum and greedy gives the
practical baseline.  The question the experiment answers is whether the
flagship instance is hard enough for the choice of optimizer to matter at all:
if greedy reaches the certified optimum at every budget, no method can be
distinguished on this instance and the benchmark's discriminating power has to
come from elsewhere.

The instance is CONJUNCTIVE -- some outcomes fire only when several inputs are
present together -- so greedy's 1-1/e submodular guarantee does not apply here.
Whether it nevertheless reaches the optimum is the measurement.

Provenance: the profile is the real pathology export described in
benchmarks/heme/QBMED-HEME-001, derived from an implementation audit of a
running rule engine.  No clinical claim follows from any result below: a
smaller covering panel does not mean a test is clinically unnecessary.

Writes: results/flagship_records.jsonl   (canonical, schema-versioned)
        results/flagship_coverage_curve.csv  (derived view for the report)
        results/flagship_env.json
"""
from __future__ import annotations

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
SEED = 42


def _flagship_instance():
    """QBMED-HEME-001 where it is available, a synthetic stand-in otherwise.

    QBMED-HEME-001 ships with this package, so the fallback normally never
    fires. A partial install or a vendored subset can trip it, and then the
    result is a real measurement of a shape-calibrated synthetic instance
    rather than a reproduction of the published curve. The instance_id in
    every output row says which one was solved.
    """
    try:
        return ins.heme_benchmark(seed=SEED), True
    except ins.HemeDomainUnavailable:
        print("QBMED-HEME-001 is not in this checkout; using the synthetic "
              "conjunctive panel instead. Numbers will not match the published "
              "curve, and the instance_id records which instance was solved.")
        return ins.conjunctive_panel(66, 88, seed=SEED), False


def main() -> None:
    t_start = time.perf_counter()
    RES.mkdir(exist_ok=True)

    inst, is_real = _flagship_instance()
    n_in = inst.n_inputs
    # The benchmark_id follows the instance actually solved. Hardcoding the
    # flagship id here once let a fallback run write a synthetic curve under
    # the real instance's name, which is exactly the confusion the fallback
    # docstring promises will not happen.
    benchmark_id = inst.instance_id

    allrecs = rs.ResultSet()
    rows = []
    for K in range(1, n_in + 1):
        spec = rn.RunSpec(benchmark_id=benchmark_id, input_budget=K,
                          benchmark_version="1.0.0",
                          notes="flagship coverage curve")
        out = rn.run_comparison(inst, spec, [{"algorithm": "ilp"},
                                             {"algorithm": "greedy"}])
        by = {r.algorithm: r for r in out}
        ilp, greedy = by["ilp"], by["greedy"]
        for r in out:
            allrecs.append(r)

        # The derived view is computed from the records rather than gathered
        # alongside them, so the CSV cannot drift from the canonical file.
        rows.append({
            "K": K,
            "ilp_cov": ilp.coverage_fraction,
            "greedy_cov": greedy.coverage_fraction,
            "ilp_cov_achievable": ilp.coverage_fraction_of_achievable,
            "ilp_obj": ilp.objective_value,
            "greedy_obj": greedy.objective_value,
            "certified": ilp.certified_optimal,
            "ilp_ms": ilp.runtime_ms,
            "greedy_ms": greedy.runtime_ms,
            "n_sel": ilp.n_selected,
            # Optimality is judged against the CERTIFIED optimum, and the gap
            # comes from the record's own reference rather than being
            # recomputed here.
            "greedy_is_optimal": bool(abs(greedy.optimality_gap_abs) < 1e-9),
            "rel_gap": greedy.optimality_gap_rel,
            "greedy_bound_applies": bool(
                greedy.extra.get("submodular_bound_applies", False)),
            # Greedy can exhaust its improving moves before exhausting its
            # budget: on a conjunctive instance the remaining outcomes need two
            # or more inputs added together, so no single input shows a
            # marginal gain and selection halts.  Extra budget then buys
            # nothing, which is why the failures cluster at LARGE K.
            "greedy_stalled": bool(
                greedy.extra.get("stalled_with_budget_remaining", False)),
            "greedy_n_selected": greedy.n_selected,
        })

    allrecs.to_jsonl(RES / "flagship_records.jsonl")
    df = pd.DataFrame(rows)
    df.to_csv(RES / "flagship_coverage_curve.csv", index=False)

    full = df[df.ilp_cov_achievable >= 1 - 1e-9]
    env = {
        "environment": rs.environment_fingerprint(),
        "benchmark_id": benchmark_id,
        "n_inputs": inst.n_inputs,
        "n_outcomes": inst.n_outcomes,
        "n_arms": inst.n_arms,
        "is_conjunctive": bool(inst.is_conjunctive),
        "instance_checksum": inst.checksum(),
        "seed": SEED,
        "provenance": ("real pathology export (implementation audit of a rule "
                       "engine)") if is_real else
                      "synthetic stand-in, shape-calibrated to the flagship",
        "schema_version": rs.SCHEMA_VERSION,
        "total_runtime_s": round(time.perf_counter() - t_start, 1),
    }
    (RES / "flagship_env.json").write_text(json.dumps(env, indent=2))
    print(f"{len(df)} budgets, {len(allrecs)} records -> {RES}")
    stalled = df[df.greedy_stalled]
    print(f"all certified: {bool(df.certified.all())}; "
          f"greedy optimal at {int(df.greedy_is_optimal.sum())}/{len(df)} budgets; "
          f"min K for full achievable coverage: "
          f"{int(full.K.min()) if len(full) else 'not reached'}")
    if len(stalled):
        print(f"greedy stalls from K={int(stalled.K.min())} at "
              f"{int(stalled.greedy_n_selected.iloc[0])} inputs / "
              f"{stalled.greedy_cov.max():.4f} coverage; "
              f"ILP reaches {df.ilp_cov.max():.4f}")


if __name__ == "__main__":
    main()
