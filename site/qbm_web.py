"""Browser bridge for the Q-BenchMed playground.

Runs inside Pyodide on the GitHub Pages site (and natively under
``site/dev_server.py`` for testing). Every number it returns comes from the
unmodified ``qbm`` package; this module only chooses what to run and turns the
results into JSON-ready dictionaries. It mirrors the Colab notebook.
"""
from __future__ import annotations

import time

from qbm import classical as cl
from qbm import constrained as cn
from qbm import instances as ins
from qbm import qubo as qb
from qbm import runner as rn
from qbm.qaoa import Qaoa

MAX_INPUTS = 12          # keeps every QAOA run within seconds in WebAssembly
# The pairwise encoding (inputs + at most a few auxiliaries, <= 15 qubits here)
# needs outcome degree <= 2, which these regimes guarantee. Other regimes need
# the slack encoding at 23-79 qubits: beyond a state-vector simulator.
WEB_REGIMES = ("degree_capped", "low_overlap")
_state: dict = {}


def _finite(value):
    return None if value is None or value != value else float(value)


def prepare(regime: str, n_inputs: int, seed: int, budget_fraction: float, depths: list[int]) -> dict:
    """Generate one instance and the shared QUBO; return the run plan."""
    if regime not in WEB_REGIMES:
        raise ValueError(f"regime must be one of {WEB_REGIMES} (the pairwise encoding needs outcome degree <= 2)")
    n_inputs = int(n_inputs)
    if not 6 <= n_inputs <= MAX_INPUTS:
        raise ValueError(f"inputs must be between 6 and {MAX_INPUTS}")
    n_outcomes = round(n_inputs * 88 / 66)
    inst = ins.generate(regime, n_inputs, n_outcomes, seed=int(seed))
    k = max(2, round(float(budget_fraction) * n_inputs))
    spec = rn.RunSpec(benchmark_id="QBM-WEB", input_budget=k, encoding="pairwise", notes="github pages playground")
    qp = qb.build_max_coverage(inst.A, inst.w, inst.c, k, encoding="pairwise", arms=inst.arms)
    plan = [{"algorithm": "qubo_exhaustive"}, {"algorithm": "greedy"},
            {"algorithm": "simulated_annealing", "seed": int(seed)}, {"algorithm": "tabu", "seed": int(seed)}]
    plan += [{"algorithm": "qaoa", "depth": int(p), "shots": 4096, "seed": int(seed), "n_restarts": 3}
             for p in sorted(set(int(d) for d in depths)) if 1 <= int(p) <= 3]
    _state.update(inst=inst, spec=spec, qp=qp, k=k, seed=int(seed), plan=plan)
    arms = inst.arms or []          # synthetic regimes are disjunctive: no rule arms
    conjunctive = sum(1 for arm in arms if len(arm[1]) > 1)
    return {
        "instance_id": inst.instance_id, "regime": regime, "n_inputs": n_inputs, "n_outcomes": n_outcomes,
        "budget": k, "qubo_variables": int(qp.n_vars), "arms": len(arms), "conjunctive_arms": conjunctive,
        "gamma_period": float(Qaoa(qp).gamma_period()),
        "plan": [_label(step) for step in plan],
    }


def _label(step: dict) -> str:
    names = {"qubo_exhaustive": "exhaustive", "simulated_annealing": "annealing"}
    if step["algorithm"] == "qaoa":
        return f"QAOA p={step['depth']}"
    return names.get(step["algorithm"], step["algorithm"])


def run_step(index: int) -> dict:
    """Run one solver from the plan against the shared QUBO.

    Exhaustive enumeration is re-run alongside each solver (milliseconds at
    these sizes) so the runner scores every answer against the certified
    optimum of the identical QUBO, exactly as in a full comparison.
    """
    step = _state["plan"][int(index)]
    plan = [step] if step["algorithm"] == "qubo_exhaustive" else [{"algorithm": "qubo_exhaustive"}, step]
    started = time.perf_counter()
    record = rn.run_comparison(_state["inst"], _state["spec"], plan)[-1]
    elapsed = (time.perf_counter() - started) * 1000
    gap = _finite(record.qubo_gap_abs)
    return {
        "method": _label(step),
        "family": "quantum (simulated)" if step["algorithm"] == "qaoa" else "classical",
        "formulation": record.formulation,
        "qubo_energy": _finite(record.qubo_energy),
        "gap": gap,
        "found_optimum": gap is not None and abs(gap) < 1e-9,
        "coverage": _finite(record.coverage_fraction),
        "selected": [int(i) for i in record.selected_inputs],
        "runtime_ms": round(elapsed, 1),
    }


def constrained_comparison(depths: list[int]) -> dict:
    """Penalty QAOA versus the penalty-free constrained mixer, against a random valid panel."""
    inst, k, seed, qp = _state["inst"], _state["k"], _state["seed"], _state["qp"]
    diag = cn.coverage_diagonal(inst.A, inst.w)
    optimum, _ = cn.optimum_at_weight(diag, k)
    p0 = cn.dicke_state(inst.n_inputs, k).probabilities()
    random_p = float(p0[diag.reshape(-1) <= optimum + 1e-9].sum())
    qubo_optimum = float(qp.all_energies()[1].min())
    rows = []
    for p in sorted(set(int(d) for d in depths)):
        if not 1 <= p <= 3:
            continue
        penalty = Qaoa(qp).run(p, shots=4096, seed=seed, n_restarts=3, optimum_energy=qubo_optimum)
        constrained = cn.ConstrainedQaoa(inst.A, inst.w, k).run(p, shots=4096, seed=seed, n_restarts=3, optimum_energy=optimum)
        for ansatz, result, qubits in (("penalty", penalty, int(qp.n_vars)), ("constrained", constrained, int(inst.n_inputs))):
            probability = float(result.optimum_probability)
            rows.append({"ansatz": ansatz, "depth": p, "qubits": qubits, "p_optimum": probability,
                         "lift": probability / random_p if random_p else None})
    return {"random_p": random_p, "rows": rows}


def heme_curve(budgets: list[int]) -> dict:
    """Certified branch-and-bound versus greedy on the real 66 x 88 instance."""
    heme = ins.heme_benchmark()
    conditional = [arm for arm in heme.arms if len(arm[1]) > 0]
    conjunctive = [arm for arm in conditional if len(arm[1]) > 1]
    rows = []
    for budget in sorted(set(int(b) for b in budgets)):
        budget = max(1, min(heme.n_inputs, budget))
        certified = cl.ilp(heme.A, heme.w, heme.c, budget, arms=heme.arms)
        greedy = cl.greedy(heme.A, heme.w, heme.c, budget, arms=heme.arms)
        rows.append({"budget": budget, "certified": float(certified.coverage_fraction), "greedy": float(greedy.coverage_fraction)})
    best = cl.ilp(heme.A, heme.w, heme.c, heme.n_inputs, arms=heme.arms)
    return {
        "instance_id": heme.instance_id, "n_inputs": int(heme.n_inputs), "n_outcomes": int(heme.n_outcomes),
        "conditional_arms": len(conditional), "conjunctive_arms": len(conjunctive),
        "ceiling": float(best.coverage_fraction), "unreachable": int(heme.n_outcomes - len(best.covered_outcomes)),
        "rows": rows,
    }
