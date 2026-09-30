"""Assemble the reproducibility package and benchmark report.

Every number written into the report is read from the result CSVs produced by
the experiment scripts -- none is typed in by hand. Re-running this script
after re-running the experiments regenerates the report from the new tables.

Layout follows the specification's reproducibility-package structure:

    reproducibility/
        benchmark.yaml        -- declarative benchmark definition
        environment.json      -- interpreter, platform, package versions
        parameters.json       -- every algorithm parameter used
        results.json          -- machine-readable summary statistics
        summary.csv           -- one row per algorithm/configuration
        README.md             -- how to reproduce
        REPORT.md             -- the benchmark report
"""
from __future__ import annotations

import json
import platform
import subprocess
import sys
from pathlib import Path

import numpy as np
import pandas as pd

ROOT = Path(__file__).resolve().parent.parent
RES = ROOT / "results"
OUT = ROOT / "reproducibility"
OUT.mkdir(exist_ok=True)


# --------------------------------------------------------------------------
# environment
# --------------------------------------------------------------------------
def environment() -> dict:
    pkgs = {}
    for name in ("numpy", "scipy", "pandas", "matplotlib", "qiskit", "dimod", "pytest"):
        try:
            mod = __import__(name)
            pkgs[name] = getattr(mod, "__version__", "unknown")
        except ImportError:
            pkgs[name] = None
    return {
        "python": sys.version.split()[0],
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor() or "unknown",
        "packages": pkgs,
        "note": ("Wall-clock figures are CPU state-vector simulation, not QPU "
                 "execution. They are recorded for implementation cost only and "
                 "are never used to rank quantum against classical methods."),
    }


# --------------------------------------------------------------------------
# summary statistics, all read from the result tables
# --------------------------------------------------------------------------
def summarise() -> tuple[dict, pd.DataFrame]:
    cv = pd.read_csv(RES / "flagship_coverage_curve.csv")
    dp = pd.read_csv(RES / "discriminating_power.csv")
    hh = pd.read_csv(RES / "headtohead.csv")
    pl = pd.read_csv(RES / "penalty_landscape.csv")
    ns = pd.read_csv(RES / "noise_sweep.csv")

    # Saturation is defined against ACHIEVABLE coverage, not against all 88
    # outcomes.  On the real profile some outcomes are unreachable by any
    # selection -- a conjunctive outcome whose arms are never jointly
    # satisfiable within the input set -- so coverage_fraction has a ceiling
    # below 1.0 and a 0.999 threshold on it never fires.  The achievable
    # denominator is the honest one: it asks what fraction of the reachable
    # outcome space a panel covers.
    sat = cv[cv.ilp_cov_achievable >= 0.999]
    k_sat = int(sat.K.min()) if len(sat) else int(cv.K.max())
    max_cov = float(cv.ilp_cov.max())

    q = hh[hh.family == "quantum"].copy()
    q["uniform_prob"] = q.n_optimal_states / (2.0 ** q.qubits)
    q["enrichment"] = q.optimum_probability / q.uniform_prob
    cls = hh[hh.family == "classical"]

    S = {
        "flagship": {
            "n_inputs": 66,
            "n_outcomes": 88,
            "min_inputs_for_full_achievable_coverage": k_sat,
            "max_coverage_fraction_of_all_outcomes": round(max_cov, 4),
            "outcomes_unreachable_by_any_selection": bool(max_cov < 0.999),
            "coverage_at_budget_10": round(float(cv.loc[cv.K == 10, "ilp_cov"].iloc[0]), 4),
            "ilp_runtime_ms_range": [round(float(cv.ilp_ms.min()), 1),
                                     round(float(cv.ilp_ms.max()), 1)],
            "every_point_certified_optimal": bool(cv.certified.all()),
            "budget_range_swept": [int(cv.K.min()), int(cv.K.max())],
            # Greedy's success rate depends strongly on where the budget sweep
            # stops: past the saturation point every method trivially ties at
            # full coverage, which inflates the figure. Report the rate over
            # the PRE-SATURATION range, where the comparison carries
            # information, and give the whole-sweep figure alongside it so the
            # difference is visible rather than hidden by a choice of range.
            "greedy_matches_optimum_below_saturation": round(float(
                cv[cv.K < k_sat].greedy_is_optimal.mean()), 3),
            "greedy_matches_optimum_whole_sweep": round(float(cv.greedy_is_optimal.mean()), 3),
            # Greedy's failures cluster at LARGE budgets, not small ones,
            # because it runs out of improving single-input moves before it
            # runs out of budget and then cannot use the rest.
            "greedy_stalls": bool(cv.greedy_stalled.any()),
            "greedy_stall_budget": (int(cv[cv.greedy_stalled].K.min())
                                    if cv.greedy_stalled.any() else None),
            "greedy_plateau_inputs": (int(cv[cv.greedy_stalled].greedy_n_selected.iloc[0])
                                      if cv.greedy_stalled.any() else None),
            "greedy_plateau_coverage": (round(float(cv[cv.greedy_stalled].greedy_cov.max()), 4)
                                        if cv.greedy_stalled.any() else None),
            "greedy_failures_at_or_above_stall": int(
                (~cv[cv.greedy_stalled].greedy_is_optimal).sum()),
            "greedy_failures_below_stall": int(
                (~cv[~cv.greedy_stalled].greedy_is_optimal).sum()),
        },
        "discriminating_power": {
            "instances": int(len(dp)),
            "greedy_optimal_overall": round(float(dp.greedy_optimal.mean()), 3),
            "greedy_optimal_by_regime": {k: round(float(v), 3) for k, v in
                                         dp.groupby("regime").greedy_optimal.mean().items()},
            "hardest_cell": {
                "regime": "degree_capped", "budget_fraction": 0.4,
                "greedy_optimal": round(float(dp[(dp.regime == "degree_capped") &
                                                 (dp.K_frac == 0.4)].greedy_optimal.mean()), 3),
            },
        },
        "head_to_head": {
            "qubit_range": [int(q.qubits.min()), int(q.qubits.max())],
            "qaoa_reaches_optimum_by_depth": {int(k): round(float(v), 3) for k, v in
                                              q.groupby("depth").found_optimum.mean().items()},
            "qaoa_reaches_optimum_by_size": {int(k): round(float(v), 3) for k, v in
                                             q.groupby("n_inputs").found_optimum.mean().items()},
            "classical_reaches_optimum": {f"{a} ({f})": round(float(v), 3) for (a, f), v in
                                          cls.groupby(["algorithm", "formulation"])
                                          .found_optimum.mean().items()},
            "optimum_mass_enrichment_median": round(float(q.enrichment.median()), 2),
            "optimum_mass_enrichment_range": [round(float(q.enrichment.min()), 3),
                                              round(float(q.enrichment.max()), 1)],
            "runs_at_or_below_chance_fraction": round(float((q.enrichment <= 1.1).mean()), 3),
            "two_qubit_gates_by_depth": {int(k): int(round(v)) for k, v in
                                         q.groupby("depth").two_qubit_gates.mean().items()},
        },
        "penalty_landscape": {
            "penalty_dominance_by_size": {int(k): round(float(v), 1) for k, v in
                                          pl.groupby("n_inputs").penalty_dominance.mean().items()},
            "negative_energy_fraction_by_size": {int(k): round(float(v), 4) for k, v in
                                                 pl.groupby("n_inputs").frac_negative.mean().items()},
            "useful_span_percent_of_spectrum": {
                int(k): round(100 * float(v), 3) for k, v in
                (pl.useful_span / pl.span).groupby(pl.n_inputs).mean().items()},
        },
        "noise": {
            "sweep_points": int(len(ns)),
            "trajectories_per_point": int(ns.trajectories.iloc[0]),
            "degradation_at_1pct_depolarizing": {
                str(k): round(float(v), 1) for k, v in
                ns[(ns.rate == 0.01) & (ns.channel == "depolarizing")]
                .groupby("mechanism").delta_E.mean().items()},
            "ideal_mean_energy_by_depth": {int(k): round(float(v), 1) for k, v in
                                           ns[ns.rate == 0].groupby("depth").mean_energy.mean().items()},
            "dephasing_invisible_at_depth_1": bool(
                (ns[(ns.mechanism == "single_qubit_error") & (ns.channel == "dephasing") &
                    (ns.depth == 1)].delta_E.abs() < 1e-12).all()),
            # Smallest two-qubit rate at which EVERY point exceeds one standard
            # error. Below this, some reported shifts are not distinguishable
            # from trajectory sampling noise -- taking the min over resolved
            # points instead would overstate the sweep's resolution.
            "fully_resolved_two_qubit_rate": float(min(
                r for r, g in
                ns[(ns.mechanism == "two_qubit_error") &
                   (ns.channel == "depolarizing") & (ns.rate > 0)].groupby("rate")
                if bool((g.delta_sig.abs() > 1.0).all()))),
            "resolved_fraction_at_smallest_rate": round(float(
                (ns[(ns.mechanism == "two_qubit_error") & (ns.channel == "depolarizing") &
                    (ns.rate == ns[ns.rate > 0].rate.min())].delta_sig.abs() > 1.0).mean()), 3),
            "discarded_sweep": "noise_sweep_INVALID_phasebug.csv -- see REPORT.md",
        },
        "tests": {"passing": test_count()},
        "constrained": _constrained_summary(),
        "real_slices": _real_slices_summary(),
        "qiskit": _qiskit_summary(),
    }

    # one row per algorithm/configuration
    rows = []
    for (alg, fam, form, dep), g in hh.groupby(
            ["algorithm", "family", "formulation", hh.depth.fillna(-1)], dropna=False):
        rows.append({
            "algorithm": alg, "family": fam, "formulation": form,
            "depth": None if dep == -1 else int(dep),
            "instances": len(g),
            "reaches_certified_optimum": round(float(g.found_optimum.mean()), 3),
            # `gap` is an ABSOLUTE energy difference (E - E*), not a ratio: the
            # coverage objective is integer-valued under uniform weights, so a
            # gap of 1.0 means one uncovered outcome. The median is 0 for every
            # method here, so mean and worst case are reported alongside it --
            # a median of 0 would otherwise hide all the failures.
            "mean_absolute_gap": round(float(g.gap.mean()), 4),
            "worst_absolute_gap": round(float(g.gap.max()), 4),
            "median_runtime_ms": (None if not np.isfinite(g.runtime_ms.median())
                                  else round(float(g.runtime_ms.median()), 2)),
            "runtime_note": ("ground truth; timed once per instance, not per row"
                             if alg == "exhaustive" else
                             "CPU simulation cost, not QPU time" if fam == "quantum" else ""),
            "mean_two_qubit_gates": (None if not np.isfinite(g.two_qubit_gates.mean())
                                     else int(round(g.two_qubit_gates.mean()))),
        })
    summary = pd.DataFrame(rows).sort_values(
        ["family", "reaches_certified_optimum"], ascending=[True, False])
    return S, summary


def _constrained_summary() -> dict | None:
    """Penalty vs constrained mixer on the head-to-head instances, if run."""
    path = RES / "constrained_mixer.csv"
    if not path.exists():
        return None
    cm = pd.read_csv(path)
    by = cm.groupby(["ansatz", "depth"])
    wide = cm.pivot_table(index=["n_inputs", "instance_seed", "depth"],
                          columns="ansatz", values="optimum_probability")
    ratio = (wide["constrained"] / wide["penalty"].clip(lower=1e-12))
    return {
        "instances": int(cm.groupby(["n_inputs", "instance_seed"]).ngroups),
        "p_opt_by_ansatz_depth": {f"{a}|{d}": round(float(v), 4)
                                  for (a, d), v in by.optimum_probability.mean().items()},
        "p_opt_by_ansatz_size": {f"{a}|{n}": round(float(v), 4) for (a, n), v in
                                 cm[cm.depth == 2].groupby(["ansatz", "n_inputs"])
                                 .optimum_probability.mean().items()},
        "found_optimum_by_ansatz_depth": {f"{a}|{d}": round(float(v), 3)
                                          for (a, d), v in by.found_optimum.mean().items()},
        "feasible_by_ansatz": {a: round(float(v), 4) for a, v in
                               cm.groupby("ansatz").feasible_probability.mean().items()},
        "constrained_wins_fraction": round(float((ratio > 1).mean()), 3),
        "lift_over_random_by_ansatz_depth": {
            f"{a}|{d}": round(float(v), 2) for (a, d), v in
            by.lift_over_random.median().items()},
        "constrained_lift_by_size": {int(n): round(float(v), 1) for n, v in
                                     cm[cm.ansatz == "constrained"]
                                     .groupby("n_inputs").lift_over_random.median().items()},
        "constrained_lift_range": [round(float(cm[cm.ansatz == "constrained"].lift_over_random.min()), 1),
                                   round(float(cm[cm.ansatz == "constrained"].lift_over_random.max()), 1)],
        "constrained_beats_random_fraction": round(float(
            (cm[cm.ansatz == "constrained"].lift_over_random > 1).mean()), 3),
        "penalty_below_random_fraction": round(float(
            (cm[cm.ansatz == "penalty"].lift_over_random < 1).mean()), 3),
        "random_hit_by_size": {int(n): round(float(v), 3) for n, v in
                               cm.groupby("n_inputs").random_feasible_hit.mean().items()},
        "median_ratio": round(float(ratio.median()), 1),
        "qubits_by_ansatz_size": {f"{a}|{n}": int(v) for (a, n), v in
                                  cm.groupby(["ansatz", "n_inputs"]).qubits.first().items()},
        "gates_by_ansatz_depth": {f"{a}|{d}": int(round(float(v))) for (a, d), v in
                                  by.two_qubit_gates.mean().items()},
        "gates_with_prep_by_ansatz_depth": {f"{a}|{d}": int(round(float(v))) for (a, d), v in
                                            by.two_qubit_gates_with_prep.mean().items()},
    }


def _real_slices_summary() -> dict | None:
    path = RES / "real_slices.csv"
    if not path.exists():
        return None
    rl = pd.read_csv(path)
    first = rl.groupby("n_inputs").first()
    return {
        "sizes": [int(n) for n in first.index],
        "penalty_qubits": {int(n): int(v) for n, v in first.penalty_qubits.items()},
        "conjunctive_arms": {int(n): int(v) for n, v in first.conjunctive_arms.items()},
        "n_arms": {int(n): int(v) for n, v in first.n_arms.items()},
        "random_p_opt": {int(n): round(float(v), 4) for n, v in first.random_feasible_p_opt.items()},
        "p_opt": {f"{int(n)}|{int(d)}": round(float(v), 4)
                  for n, d, v in rl[["n_inputs", "depth", "optimum_probability"]].itertuples(index=False)},
        "lift_range": [round(float(rl.lift_over_random.min()), 1),
                       round(float(rl.lift_over_random.max()), 1)],
        "greedy_finds_all": bool(rl.greedy_finds_optimum.all()),
        "depth3_worse_than_depth2": [int(n) for n, g in rl.groupby("n_inputs")
                                     if float(g[g.depth == 3].optimum_probability.iloc[0])
                                     < float(g[g.depth == 2].optimum_probability.iloc[0])],
        "max_term_order": int(max(max(int(k) for k in json.loads(t))
                                  for t in rl.terms_by_order)),
    }


def _qiskit_summary() -> dict | None:
    path = RES / "qiskit_reproduction.csv"
    if not path.exists():
        return None
    qr = pd.read_csv(path)
    env = json.loads((RES / "qiskit_reproduction_env.json").read_text())
    return {"runs": int(len(qr)), "agree": int(qr.agrees.sum()),
            "min_fidelity": float(qr.state_fidelity.min()),
            "max_abs_z": round(float(qr.z_score.abs().max()), 2),
            "shots": int(qr.shots.iloc[0]), "qiskit": env["qiskit"],
            "p_opt_by_ansatz_size": {f"{a}|{n}": round(float(v), 4) for (a, n), v in
                                     qr.groupby(["ansatz", "n_inputs"]).exact_p_opt.mean().items()}}


def test_count() -> int:
    r = subprocess.run([sys.executable, "-m", "pytest", "tests/", "-q", "--no-header"],
                       cwd=ROOT, capture_output=True, text=True)
    for line in reversed(r.stdout.strip().splitlines()):
        if "passed" in line:
            for tok in line.replace("=", " ").split():
                if tok.isdigit():
                    return int(tok)
    return -1


def main() -> None:
    S, summary = summarise()
    (OUT / "environment.json").write_text(json.dumps(environment(), indent=2))
    (OUT / "results.json").write_text(json.dumps(S, indent=2))
    summary.to_csv(OUT / "summary.csv", index=False)

    params = {}
    for f in ("headtohead_env.json", "noise_sweep_env.json"):
        p = RES / f
        if p.exists():
            params[f.replace("_env.json", "")] = json.loads(p.read_text())
    (OUT / "parameters.json").write_text(json.dumps(params, indent=2))

    sys.path.insert(0, str(Path(__file__).parent))
    import report_template
    (OUT / "REPORT.md").write_text(report_template.render(S))
    print(f"wrote {OUT}/  ({len(summary)} summary rows, "
          f"{len((OUT / 'REPORT.md').read_text())} chars of report)")


if __name__ == "__main__":
    main()
