"""The common experiment runner: one entry point, one record format.

Spec section 50 requires that every algorithm receive the same problem
instance, the same objective, the same constraints and the same preprocessing.
The only way to make that true rather than intended is to have a single code
path construct the problem and hand it to whichever solver is being measured.
That is this module: `run_one` builds the instance's objective once, dispatches
to one algorithm, and returns a :class:`~qbm.results.RunRecord`.  No solver in
`qbm.classical` or `qbm.qaoa` ever sees a differently-prepared problem.

Two consequences are worth naming.

*Every algorithm is scored by the native objective.*  A QUBO or QAOA answer is
decoded to a selection vector and then re-scored through the same native
function the classical solvers use, so an infeasible QUBO optimum is reported
as infeasible instead of being credited with its penalised energy.

*QAOA is not given ground truth it could not have.*  ``optimum_energy`` for the
sampling metrics is supplied only when an exact solver has certified it on the
same QUBO; otherwise the probability-of-optimum metrics are computed against
QAOA's own best sample and flagged as such in `sampling_metrics`.

Both comparison axes are filled from the same run.  Every record carries the
native objective of its decoded selection *and* the energy of that selection in
the QUBO under test -- for a native-formulation solver such as greedy, the
energy of its selection's best completion.  That is what lets a QUBO-axis
experiment place greedy alongside the QUBO solvers without re-deriving anything
or changing what greedy does.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Callable, Iterable, Sequence

import numpy as np

from . import classical as cl
from . import qubo as qb
from .instances import Instance
from .results import ResultSet, RunRecord

ALGORITHM_VERSION = "qbm-0.1.0"

#: Algorithms that return a certificate of optimality (used for the reference).
EXACT_ALGORITHMS = frozenset({"exhaustive", "ilp", "qubo_exhaustive"})

#: Algorithms whose runtime is simulated quantum time and therefore not
#: comparable with classical wall clock (spec section 50).
SIMULATED_QUANTUM = frozenset({"qaoa"})


@dataclass
class RunSpec:
    """The formulation every algorithm in one comparison must share."""

    benchmark_id: str
    input_budget: int
    objective: str = "maximum_outcome_coverage"
    alpha: float = 1.0
    beta: float = 0.0
    gamma: float | None = None
    lam: float | None = None
    encoding: str = "slack"
    weighting: str = "uniform"
    cost_model: str = "uniform"
    benchmark_version: str = "unversioned"
    coverage_target: float | None = None
    notes: str = ""

    def penalties(self) -> dict:
        return {"alpha": self.alpha, "beta": self.beta,
                "gamma": self.gamma, "lam": self.lam}


def _instance_facts(inst: Instance) -> dict:
    reachable = int(sum(1 for j in range(inst.n_outcomes)
                        if inst.A[:, j].any() or _is_unconditional(inst, j)))
    n_uncond = (0 if inst.arms is None
                else len({j for j, mem in inst.arms if len(mem) == 0}))
    return {
        "instance_id": inst.instance_id,
        "instance_checksum": inst.checksum(),
        "regime": inst.regime,
        "n_inputs": inst.n_inputs,
        "n_outcomes": inst.n_outcomes,
        "n_arms": inst.n_arms,
        "is_conjunctive": bool(inst.is_conjunctive),
        "n_unconditional_outcomes": n_uncond,
        "n_reachable_outcomes": reachable,
    }


def _is_unconditional(inst: Instance, j: int) -> bool:
    if inst.arms is None:
        return False
    return any(jj == j and len(mem) == 0 for jj, mem in inst.arms)


def achievable_coverage_weight(inst: Instance) -> float:
    """Total weight any selection could possibly reach.

    An outcome with no arm is unreachable, so full coverage of the outcome
    space is unattainable by construction.  Reporting coverage only as a
    fraction of *all* outcomes would make every algorithm look bounded away
    from the optimum for a reason that has nothing to do with the algorithm;
    both denominators are therefore recorded.
    """
    mask = qb.coverage_mask(inst.A, np.ones(inst.n_inputs, bool), inst.arms)
    return float(inst.w[mask].sum())


# ------------------------------------------------------------------ dispatch
def _run_classical(name: str, inst: Instance, spec: RunSpec, seed: int | None,
                   params: dict) -> tuple[cl.Result, dict, int]:
    A, w, c, K = inst.A, inst.w, inst.c, spec.input_budget
    shared = dict(alpha=spec.alpha, beta=spec.beta, arms=inst.arms)
    if name == "exhaustive":
        res = cl.exhaustive(A, w, c, K, **shared, **params)
        n_vars = inst.n_inputs
    elif name == "ilp":
        res = cl.ilp(A, w, c, K, **shared, **params)
        n_vars = inst.n_inputs + inst.n_outcomes
    elif name == "greedy":
        res = cl.greedy(A, w, c, K, **shared, **params)
        n_vars = inst.n_inputs
    elif name == "qubo_exhaustive":
        res = _qubo_exhaustive(inst, spec, params)
        n_vars = res.n_vars
    elif name in ("simulated_annealing", "tabu"):
        res = cl.solve_via_qubo(A, w, c, K, method=name, encoding=spec.encoding,
                               seed=seed, **shared, **params)
        n_vars = res.n_vars
    else:
        raise KeyError(f"unknown algorithm {name!r}")
    return res, {}, n_vars


def _run_qaoa(inst: Instance, spec: RunSpec, seed: int | None, params: dict,
              reference_energy: float | None) -> tuple[cl.Result, dict, int]:
    from .qaoa import Qaoa

    depth = int(params.pop("depth", 1))
    shots = int(params.pop("shots", 10_000))
    memory_limit_bytes = params.pop("memory_limit_bytes", None)

    t0 = time.perf_counter()
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, spec.input_budget,
                              encoding=spec.encoding, alpha=spec.alpha,
                              beta=spec.beta, gamma=spec.gamma, lam=spec.lam,
                              arms=inst.arms)
    t_build = (time.perf_counter() - t0) * 1e3

    engine = Qaoa(q, memory_limit_bytes=memory_limit_bytes)

    def feasible_fn(z):
        x = q.varmap.x_of(z)
        return int(x.sum()) <= spec.input_budget

    outcome = engine.run(depth, shots=shots, seed=seed,
                         optimum_energy=reference_energy,
                         feasible_fn=feasible_fn, **params)

    # The delivered answer is the best *feasible* sample.  The lowest-energy
    # sample can break the budget, and crediting QAOA with that selection's
    # unpenalised native objective would score a constraint violation as a
    # better-than-optimal result.  When no sample is feasible the raw best is
    # reported and the record's `feasible` flag carries the failure.
    delivered = outcome.best_feasible_state or outcome.best_sampled_state
    x = q.varmap.x_of(np.asarray(delivered, dtype=np.uint8))
    res = cl._summarise(inst.A, inst.w, inst.c, x, spec.input_budget, "qaoa",
                        outcome.runtime_ms, alpha=spec.alpha, beta=spec.beta,
                        seed=seed, formulation="qubo", encoding=spec.encoding,
                        arms=inst.arms,
                        # The energy of the *delivered* assignment, so the QUBO
                        # axis and the native axis describe the same answer.
                        # The raw minimum survives in `sampling_metrics`.
                        qubo_energy=(outcome.best_feasible_energy
                                     if outcome.best_feasible_state
                                     else outcome.best_sampled_energy),
                        delivered_sample="best_feasible"
                        if outcome.best_feasible_state else "best_energy",
                        no_feasible_sample=outcome.best_feasible_state is None,
                        final_expectation=outcome.final_expectation)
    res.n_vars = q.n_vars

    quantum = {
        "resources": outcome.resources,
        "optimizer": outcome.optimizer.as_dict(),
    }
    sampling = {
        "best_sampled_energy": outcome.best_sampled_energy,
        "best_sample_probability": outcome.best_sample_probability,
        "best_sample_feasible": outcome.best_sample_feasible,
        "best_feasible_energy": outcome.best_feasible_energy,
        "optimum_probability": outcome.optimum_probability,
        "near_optimum_probability": outcome.near_optimum_probability,
        "feasible_probability": outcome.feasible_probability,
        "final_expectation": outcome.final_expectation,
        # Without a certified optimum the probability metrics are measured
        # against QAOA's own best sample, which flatters QAOA; say so.
        "measured_against_certified_optimum": reference_energy is not None,
    }
    return res, {"quantum": quantum, "sampling": sampling,
                 "timings": {"qubo_build": t_build,
                             "parameter_optimization": outcome.optimizer.runtime_ms,
                             "sampling_and_scoring":
                                 outcome.runtime_ms - outcome.optimizer.runtime_ms},
                 "depth": depth, "shots": shots}, q.n_vars


def _qubo_exhaustive(inst: Instance, spec: RunSpec, params: dict) -> cl.Result:
    """Enumerate the QUBO under test and return its ground state as a selection.

    Distinct from `exhaustive`, which enumerates the 2^n *selections* of the
    native problem.  This enumerates the 2^m assignments of the QUBO, auxiliary
    variables included, and so certifies the object QAOA actually samples.  On
    an exact encoding the two agree, and a disagreement is a formulation bug
    worth surfacing rather than a tie worth reporting -- so the decoded ground
    state is scored through the same native objective as everything else.
    """
    t0 = time.perf_counter()
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, spec.input_budget,
                              encoding=spec.encoding, alpha=spec.alpha,
                              beta=spec.beta, gamma=spec.gamma, lam=spec.lam,
                              arms=inst.arms)
    Z, E = q.all_energies()
    k = int(np.argmin(E))
    ground = float(E[k])
    x = q.varmap.x_of(Z[k])
    dt = (time.perf_counter() - t0) * 1e3
    res = cl._summarise(inst.A, inst.w, inst.c, x, spec.input_budget,
                        "qubo_exhaustive", dt, alpha=spec.alpha, beta=spec.beta,
                        formulation="qubo", encoding=spec.encoding,
                        arms=inst.arms,
                        # Certified on the QUBO axis always; on the native axis
                        # only when the encoding is exact, since a compact
                        # encoding's ground state can decode to a sub-optimal
                        # selection.
                        certified_optimal=spec.encoding in ("slack", "pairwise"),
                        qubo_certified=True,
                        n_optimal_states=int(np.sum(E <= ground + 1e-9)),
                        optimum_density=float(np.mean(E <= ground + 1e-9)))
    res.n_vars = q.n_vars
    res.extra["qubo_energy"] = ground
    return res


CLASSICAL = ("exhaustive", "ilp", "greedy", "qubo_exhaustive",
             "simulated_annealing", "tabu")


def run_one(inst: Instance, algorithm: str, spec: RunSpec, *, seed: int | None = None,
            reference_energy: float | None = None, **params) -> RunRecord:
    """Run one algorithm on one instance and return a complete record."""
    t_total = time.perf_counter()
    params = dict(params)

    if algorithm == "qaoa":
        res, aux, n_vars = _run_qaoa(inst, spec, seed, params, reference_energy)
    elif algorithm in CLASSICAL:
        res, aux, n_vars = _run_classical(algorithm, inst, spec, seed, params)
    else:
        raise KeyError(f"unknown algorithm {algorithm!r}; "
                       f"known: {sorted(CLASSICAL) + ['qaoa']}")
    total_ms = (time.perf_counter() - t_total) * 1e3

    achievable = achievable_coverage_weight(inst)
    cov_weight = float(inst.w[[j in set(res.covered_outcomes)
                               for j in range(inst.n_outcomes)]].sum())

    timings = {"total": total_ms, "algorithm": res.runtime_ms}
    timings.update(aux.get("timings", {}))

    # Place every algorithm on the QUBO axis, including the ones that never
    # touched a QUBO: a native solver's selection is lifted to its best
    # completion.  This is what makes a QUBO-energy comparison able to include
    # greedy without greedy having solved a QUBO.
    qubo_energy = res.extra.pop("qubo_energy", None)
    if qubo_energy is None:
        qubo_energy = _lift_to_qubo(inst, spec, res.selected_inputs)

    rec = RunRecord(
        benchmark_id=spec.benchmark_id,
        algorithm=algorithm,
        algorithm_version=ALGORITHM_VERSION,
        benchmark_version=spec.benchmark_version,
        objective=spec.objective,
        input_budget=spec.input_budget,
        coverage_target=spec.coverage_target,
        weighting=spec.weighting,
        cost_model=spec.cost_model,
        formulation=res.formulation,
        encoding=res.encoding,
        penalties=spec.penalties(),
        n_problem_variables=int(n_vars),
        algorithm_params={k: v for k, v in params.items()
                          if k != "memory_limit_bytes"},
        seed=seed,
        backend="statevector" if algorithm == "qaoa" else None,
        qaoa_depth=aux.get("depth"),
        shots=aux.get("shots"),
        quantum_resources=aux.get("quantum", {}),
        sampling_metrics=aux.get("sampling", {}),
        objective_value=res.objective_value,
        selected_inputs=res.selected_inputs,
        covered_outcomes=res.covered_outcomes,
        n_selected=len(res.selected_inputs),
        coverage_weight=cov_weight,
        coverage_fraction=res.coverage_fraction,
        coverage_fraction_of_achievable=(cov_weight / achievable
                                         if achievable > 0 else float("nan")),
        input_cost=res.input_cost,
        feasible=res.feasible,
        certified_optimal=bool(res.extra.get("certified_optimal", False)),
        qubo_energy=qubo_energy,
        qubo_certified=bool(res.extra.get("qubo_certified", False)),
        runtime_ms=res.runtime_ms,
        timings_ms=timings,
        wall_clock_comparable=algorithm not in SIMULATED_QUANTUM,
        notes=spec.notes,
        extra=dict(res.extra),
        **_instance_facts(inst),
    )
    return rec


def run_comparison(inst: Instance, spec: RunSpec, plan: Sequence[dict], *,
                   attach_references: bool = True,
                   progress: Callable[[str], None] | None = None) -> ResultSet:
    """Run a list of ``{"algorithm": ..., "seed": ..., ...}`` entries in order.

    Exact algorithms are run first when present, so their certified optimum is
    available as ``reference_energy`` for the QAOA sampling metrics that follow
    -- the same ground truth, not a separately-derived one.
    """
    ordered = ([e for e in plan if e.get("algorithm") in EXACT_ALGORITHMS]
               + [e for e in plan if e.get("algorithm") not in EXACT_ALGORITHMS])
    out = ResultSet()
    qubo_reference: float | None = None
    for entry in ordered:
        entry = dict(entry)
        algo = entry.pop("algorithm")
        seed = entry.pop("seed", None)
        if progress:
            progress(f"{algo}"
                     + (f" depth={entry['depth']}" if "depth" in entry else "")
                     + (f" seed={seed}" if seed is not None else ""))
        rec = run_one(inst, algo, spec, seed=seed,
                      reference_energy=qubo_reference, **entry)
        out.append(rec)
        if rec.certified_optimal and qubo_reference is None:
            # QAOA's probability-of-optimum is a statement about the QUBO's
            # ground state, so the reference handed to it must be an energy in
            # that QUBO -- not the native optimum, which lives on another axis.
            qubo_reference = rec.qubo_energy
    if attach_references:
        out.attach_references()
        out.attach_qubo_references()
    return out


def _lift_to_qubo(inst: Instance, spec: RunSpec,
                  selected: Sequence[int]) -> float | None:
    """Energy of a selection's best completion in the QUBO under test.

    Returns None when the configured encoding admits no exact completion (the
    compact `penalty` encoding), rather than a number that would not mean what
    the column says it means.
    """
    if spec.encoding not in ("slack", "pairwise"):
        return None
    try:
        q = qb.build_max_coverage(inst.A, inst.w, inst.c, spec.input_budget,
                                  encoding=spec.encoding, alpha=spec.alpha,
                                  beta=spec.beta, gamma=spec.gamma,
                                  lam=spec.lam, arms=inst.arms)
        x = np.zeros(inst.n_inputs, bool)
        x[list(selected)] = True
        return float(qb.best_completion_energy(q, x))
    except ValueError:
        return None


# --------------------------------------------------------------- noise sweep
def run_noise_sweep(inst: Instance, spec: RunSpec, *, depth: int,
                    points: Sequence[dict], trajectories: int = 500,
                    seed: int = 0, n_restarts: int = 4,
                    memory_limit_bytes: int | None = None,
                    progress: Callable[[str], None] | None = None) -> ResultSet:
    """QAOA under a sequence of noise models, at fixed parameters.

    A noise sweep is not a sequence of independent runs and is deliberately not
    expressed as repeated `run_one` calls.  Parameters are optimised **once** on
    the ideal objective and then frozen across every point (spec section 47):
    re-optimising per noise level would measure noise-aware training, a
    different and far more expensive question, and would hide the degradation
    the experiment exists to measure.  Freezing them is only meaningful if one
    function owns the optimisation, which is why it lives here.

    Each entry of `points` is a :class:`~qbm.noise.NoiseModel` keyword dict,
    e.g. ``{"channel": "dephasing", "two_qubit_error": 0.01}``.  The ideal point
    is produced by passing an empty dict, so it travels the same code path as
    the noisy ones and the comparison is not confounded by a different route
    through the simulator.

    Every record is a full :class:`~qbm.results.RunRecord`: the lowest-energy
    trajectory is decoded and scored through the same native objective as every
    classical solver, and the trajectory distribution is carried in
    `sampling_metrics`.  P(optimum) has a resolution floor of 1/trajectories,
    recorded alongside it, because a reported 0.0 means "below the floor" and
    not "measured to be zero".
    """
    from . import noise as nz
    from .qaoa import Qaoa, ising_resource_cost

    def qa_resources(engine, p):
        return ising_resource_cost(engine.h, engine.J, p,
                                   count_state_prep=engine.count_state_prep)

    t0 = time.perf_counter()
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, spec.input_budget,
                              encoding=spec.encoding, alpha=spec.alpha,
                              beta=spec.beta, gamma=spec.gamma, lam=spec.lam,
                              arms=inst.arms)
    t_build = (time.perf_counter() - t0) * 1e3
    engine = Qaoa(q, memory_limit_bytes=memory_limit_bytes)

    _, E = q.all_energies()
    ground = float(E.min())

    t0 = time.perf_counter()
    opt_rec = engine.optimize(depth, seed=seed, n_restarts=n_restarts)
    t_opt = (time.perf_counter() - t0) * 1e3
    params = np.asarray(opt_rec.final_parameters)

    out = ResultSet()
    facts = _instance_facts(inst)
    achievable = achievable_coverage_weight(inst)

    for point in points:
        model = nz.NoiseModel(**point)
        if progress:
            progress(f"depth={depth} {model.channel} {point}")
        t0 = time.perf_counter()
        rep = nz.noisy_qaoa_energies(engine, params, model,
                                     trajectories=trajectories,
                                     seed=1000 + seed, optimum_energy=ground)
        dt = (time.perf_counter() - t0) * 1e3

        x = q.varmap.x_of(np.asarray(rep.best_state, dtype=np.uint8))
        res = cl._summarise(inst.A, inst.w, inst.c, x, spec.input_budget,
                            "qaoa", dt, alpha=spec.alpha, beta=spec.beta,
                            seed=seed, formulation="qubo",
                            encoding=spec.encoding, arms=inst.arms)
        cov_weight = float(inst.w[[j in set(res.covered_outcomes)
                                   for j in range(inst.n_outcomes)]].sum())
        rec = RunRecord(
            benchmark_id=spec.benchmark_id, algorithm="qaoa",
            algorithm_version=ALGORITHM_VERSION,
            benchmark_version=spec.benchmark_version, objective=spec.objective,
            input_budget=spec.input_budget, weighting=spec.weighting,
            cost_model=spec.cost_model, formulation="qubo",
            encoding=spec.encoding, penalties=spec.penalties(),
            n_problem_variables=q.n_vars, seed=seed,
            backend="statevector", qaoa_depth=depth,
            shots=trajectories, noise_model=model.as_dict(),
            trajectories=trajectories,
            algorithm_params={"n_restarts": n_restarts},
            quantum_resources={"resources": {
                **qa_resources(engine, depth),
                "shots": trajectories,
                "optimizer_iterations": opt_rec.iterations,
            }},
            sampling_metrics={
                "mean_energy": rep.mean_energy,
                "sem_energy": rep.sem_energy,
                "best_energy": rep.best_energy,
                "optimum_probability": rep.optimum_probability,
                # A probability estimated from N trajectories cannot resolve
                # anything below 1/N; reporting the floor keeps a 0.0 from
                # being read as a measurement.
                "optimum_probability_floor": 1.0 / trajectories,
                "ideal_expectation": opt_rec.best_expectation,
                "realised_error_count": rep.realised_error_count,
                "measured_against_certified_optimum": True,
            },
            objective_value=res.objective_value,
            selected_inputs=res.selected_inputs,
            covered_outcomes=res.covered_outcomes,
            n_selected=len(res.selected_inputs),
            coverage_weight=cov_weight,
            coverage_fraction=res.coverage_fraction,
            coverage_fraction_of_achievable=(cov_weight / achievable
                                             if achievable > 0 else float("nan")),
            input_cost=res.input_cost, feasible=res.feasible,
            qubo_energy=rep.best_energy,
            runtime_ms=dt,
            timings_ms={"total": dt + t_build + t_opt, "algorithm": dt,
                        "qubo_build": t_build, "parameter_optimization": t_opt},
            wall_clock_comparable=False,
            notes=spec.notes, **facts)
        # Ground truth here is the enumerated QUBO ground state, which is a
        # stronger reference than the sweep's own best point.
        rec.attach_qubo_reference(ground, "qubo_exhaustive")
        out.append(rec)
    return out
