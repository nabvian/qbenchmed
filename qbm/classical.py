"""Classical baselines for Q-BenchMed.

Five solvers, deliberately spanning two different notions of "the problem"
(spec §16, §50):

*Native-formulation solvers* work on the constrained problem directly.

``exhaustive``
    Enumerates every input subset.  Ground truth, but only to ~24 inputs.

``ilp``
    Branch-and-bound via SciPy/HiGHS on the native MILP.  This is what provides
    *certified* optima at 66 inputs, where enumeration is impossible.  The spec
    (§16.1, §17) treats exhaustive search as the sole source of ground truth;
    that conflates tractability of enumeration with tractability of the problem.
    Maximum coverage under a cardinality budget is an integer program that HiGHS
    solves to proven optimality at this size in milliseconds.

``greedy``
    Marginal-gain selection.  Because weighted coverage is submodular and
    monotone under a cardinality constraint, this carries the classical
    ``1 - 1/e`` guarantee (Nemhauser-Wolsey-Fisher).  The bound is *recorded*
    with every result so a reported gap can be compared against it.

*QUBO solvers* work on the identical Q matrix the QAOA consumes.

``simulated_annealing``
    Geometric cooling, single-bit flips.

``tabu``
    Steepest-descent with a tabu list and random restarts.

The split matters for fairness: a QUBO solver and QAOA see byte-identical
input, so any difference between them is attributable to the optimiser.  A
native solver sees a tighter formulation, and comparing it to QAOA measures
formulation *and* optimiser together.  Both comparisons are informative; the
benchmark reports them separately and never averages across the boundary.

Every solver returns a :class:`Result` carrying the fields spec §63 requires.
"""

from __future__ import annotations

import itertools
import time
from dataclasses import dataclass, field, asdict

import numpy as np

from . import qubo as qb


# --------------------------------------------------------------------------
@dataclass
class Result:
    """Common result object (spec §63)."""

    algorithm: str
    objective_value: float          # minimisation sense, native formulation
    selected_inputs: list[int]
    covered_outcomes: list[int]
    coverage_fraction: float
    input_cost: float
    feasible: bool
    runtime_ms: float
    seed: int | None = None
    n_vars: int = 0
    formulation: str = "native"     # "native" | "qubo"
    encoding: str | None = None
    extra: dict = field(default_factory=dict)

    def as_dict(self) -> dict:
        return asdict(self)


def _summarise(A, w, c, x, K, algorithm, runtime_ms, *, alpha=1.0, beta=0.0,
               seed=None, formulation="native", encoding=None, **extra) -> Result:
    """Build a Result from a selection vector, scoring it natively.

    Every solver's answer is scored by the *same* native objective regardless of
    what formulation it optimised, so results are directly comparable.
    """
    x = np.asarray(x, dtype=bool)
    A = np.asarray(A)
    w = np.asarray(w, dtype=float)
    c = np.asarray(c, dtype=float)
    obj, feasible = qb.native_max_coverage_objective(
        A, w, c, x, K, alpha=alpha, beta=beta)
    covered = (A[x].sum(axis=0) > 0) if x.any() else np.zeros(A.shape[1], bool)
    return Result(
        algorithm=algorithm,
        objective_value=float(obj),
        selected_inputs=sorted(int(i) for i in np.flatnonzero(x)),
        covered_outcomes=sorted(int(j) for j in np.flatnonzero(covered)),
        coverage_fraction=float(w[covered].sum() / w.sum()) if w.sum() else 0.0,
        input_cost=float(c[x].sum()),
        feasible=bool(feasible),
        runtime_ms=float(runtime_ms),
        seed=seed,
        formulation=formulation,
        encoding=encoding,
        extra=extra,
    )


# --------------------------------------------------------------------------
# native-formulation solvers
# --------------------------------------------------------------------------
def exhaustive(A, w, c, K, *, alpha=1.0, beta=0.0, max_inputs=24) -> Result:
    """Enumerate every subset of inputs.  Ground truth for small instances."""
    A = np.asarray(A); w = np.asarray(w, float); c = np.asarray(c, float)
    n_in = A.shape[0]
    if n_in > max_inputs:
        raise ValueError(
            f"exhaustive search over 2^{n_in} subsets refused; use ilp() for "
            "certified optima at this size")
    t0 = time.perf_counter()
    best_obj, best_x = np.inf, np.zeros(n_in, bool)
    n_eval = 0
    for bits in itertools.product((0, 1), repeat=n_in):
        x = np.array(bits, dtype=bool)
        if x.sum() > K:
            continue
        n_eval += 1
        obj, _ = qb.native_max_coverage_objective(A, w, c, x, K,
                                                  alpha=alpha, beta=beta)
        if obj < best_obj:
            best_obj, best_x = obj, x
    dt = (time.perf_counter() - t0) * 1e3
    return _summarise(A, w, c, best_x, K, "exhaustive", dt,
                      alpha=alpha, beta=beta, certified_optimal=True,
                      subsets_evaluated=n_eval)


def ilp(A, w, c, K, *, alpha=1.0, beta=0.0, time_limit=300.0) -> Result:
    """Certified optimum of the native MILP via SciPy/HiGHS.

    Variables ``[x (n_in), y (n_out)]``, all integral in [0, 1]::

        min   -alpha * w^T y + beta * c^T x
        s.t.  y_j - sum_i A_ij x_i <= 0     (coverage linking)
              sum_i x_i <= K                (budget)

    The linking constraint is the linear form of the same relation the QUBO
    encodes with slack bits; no penalty coefficients are involved, which is why
    this is a *certified* optimum rather than a heuristic one.
    """
    from scipy.optimize import linprog, milp, Bounds, LinearConstraint

    A = np.asarray(A, float); w = np.asarray(w, float); c = np.asarray(c, float)
    n_in, n_out = A.shape
    n = n_in + n_out

    cost = np.concatenate([beta * c, -alpha * w])

    # y_j - sum_i A_ij x_i <= 0
    link = np.zeros((n_out, n))
    link[:, :n_in] = -A.T
    link[:, n_in:] = np.eye(n_out)
    budget = np.zeros((1, n))
    budget[0, :n_in] = 1.0

    constraints = [
        LinearConstraint(link, -np.inf, 0.0),
        LinearConstraint(budget, -np.inf, float(K)),
    ]
    t0 = time.perf_counter()
    res = milp(c=cost, constraints=constraints,
               integrality=np.ones(n), bounds=Bounds(0, 1),
               options={"time_limit": time_limit})
    dt = (time.perf_counter() - t0) * 1e3

    if res.x is None:
        raise RuntimeError(f"MILP did not return a solution: {res.message}")
    x = res.x[:n_in] > 0.5
    # res.status == 0 means proven optimal; anything else is a bound, not a proof
    return _summarise(A, w, c, x, K, "ilp", dt, alpha=alpha, beta=beta,
                      certified_optimal=bool(res.status == 0),
                      mip_gap=float(getattr(res, "mip_gap", 0.0) or 0.0),
                      solver_status=int(res.status), solver_message=str(res.message))


def greedy(A, w, c, K, *, alpha=1.0, beta=0.0) -> Result:
    """Marginal-gain selection with the submodular approximation bound recorded.

    Weighted coverage is monotone submodular, so under a pure cardinality
    constraint greedy achieves at least ``1 - 1/e`` (~63.2%) of the optimum.
    That bound is on the *coverage* term; with a non-zero cost term (beta > 0)
    the objective is no longer monotone and the guarantee does not apply, which
    is recorded explicitly rather than left implied.
    """
    A = np.asarray(A); w = np.asarray(w, float); c = np.asarray(c, float)
    n_in, n_out = A.shape
    t0 = time.perf_counter()
    x = np.zeros(n_in, bool)
    covered = np.zeros(n_out, bool)
    trace = []
    for _ in range(min(K, n_in)):
        gains = np.where(
            x, -np.inf,
            (A.astype(bool) & ~covered[None, :]) @ w * alpha - beta * c)
        best = int(np.argmax(gains))
        if gains[best] <= 0:
            break                      # no positive-gain input remains
        x[best] = True
        covered |= A[best].astype(bool)
        trace.append((best, float(gains[best])))
    dt = (time.perf_counter() - t0) * 1e3
    return _summarise(A, w, c, x, K, "greedy", dt, alpha=alpha, beta=beta,
                      submodular_bound_applies=bool(beta == 0.0),
                      approximation_ratio_bound=(1 - 1 / np.e) if beta == 0 else None,
                      selection_trace=trace)


# --------------------------------------------------------------------------
# QUBO-formulation solvers -- these see exactly what QAOA sees
# --------------------------------------------------------------------------
def _qubo_energy_delta(Q: np.ndarray, z: np.ndarray, i: int) -> float:
    """Energy change from flipping bit ``i``, in O(n) rather than O(n^2)."""
    s = 1.0 - 2.0 * z[i]                       # +1 if turning on, -1 if off
    lin = Q[i, i]
    inter = Q[i, :] @ z + Q[:, i] @ z - 2.0 * Q[i, i] * z[i]
    return s * (lin + inter)


def simulated_annealing(Q_or_qubo, *, n_sweeps=2000, t0=None, t1=1e-3,
                        seed=0, n_restarts=1) -> tuple[np.ndarray, float, dict]:
    """Geometric-cooling SA on a QUBO.  Returns (best_z, best_energy, meta).

    Operates on the QUBO matrix, so it and QAOA solve the byte-identical
    mathematical problem (spec §2.2, §16.4).
    """
    q = Q_or_qubo if isinstance(Q_or_qubo, qb.Qubo) else qb.Qubo.from_dense(Q_or_qubo)
    Q = q.Q
    n = q.n_vars
    rng = np.random.default_rng(seed)
    # a sensible starting temperature is the scale of a typical single flip
    if t0 is None:
        t0 = float(np.abs(np.diag(Q)).mean() + np.abs(np.triu(Q, 1)).sum() / max(n, 1))
        t0 = max(t0, 1.0)
    best_z, best_e = None, np.inf
    n_accept = 0
    for r in range(n_restarts):
        z = rng.integers(0, 2, size=n).astype(np.float64)
        e = q.energy(z)
        cur_best_z, cur_best_e = z.copy(), e
        for s in range(n_sweeps):
            T = t0 * (t1 / t0) ** (s / max(n_sweeps - 1, 1))
            for i in rng.permutation(n):
                d = _qubo_energy_delta(Q, z, int(i))
                if d <= 0 or rng.random() < np.exp(-d / T):
                    z[i] = 1.0 - z[i]
                    e += d
                    n_accept += 1
                    if e < cur_best_e:
                        cur_best_e, cur_best_z = e, z.copy()
        if cur_best_e < best_e:
            best_e, best_z = cur_best_e, cur_best_z
    return best_z.astype(np.uint8), float(best_e), {
        "n_sweeps": n_sweeps, "n_restarts": n_restarts, "t0": t0, "t1": t1,
        "accept_count": int(n_accept)}


def tabu(Q_or_qubo, *, n_iters=2000, tenure=None, seed=0, n_restarts=4
         ) -> tuple[np.ndarray, float, dict]:
    """Steepest-descent with a tabu list and random restarts."""
    q = Q_or_qubo if isinstance(Q_or_qubo, qb.Qubo) else qb.Qubo.from_dense(Q_or_qubo)
    Q = q.Q
    n = q.n_vars
    tenure = max(3, n // 4) if tenure is None else tenure
    rng = np.random.default_rng(seed)
    best_z, best_e = None, np.inf
    for r in range(n_restarts):
        z = rng.integers(0, 2, size=n).astype(np.float64)
        e = q.energy(z)
        tabu_until = np.zeros(n, dtype=int)
        cur_best_z, cur_best_e = z.copy(), e
        for it in range(n_iters):
            deltas = np.array([_qubo_energy_delta(Q, z, i) for i in range(n)])
            allowed = tabu_until <= it
            # aspiration: a tabu move is permitted if it beats the best known
            allowed |= (e + deltas) < cur_best_e - 1e-12
            if not allowed.any():
                break
            cand = np.where(allowed, deltas, np.inf)
            i = int(np.argmin(cand))
            z[i] = 1.0 - z[i]
            e += deltas[i]
            tabu_until[i] = it + tenure
            if e < cur_best_e:
                cur_best_e, cur_best_z = e, z.copy()
        if cur_best_e < best_e:
            best_e, best_z = cur_best_e, cur_best_z
    return best_z.astype(np.uint8), float(best_e), {
        "n_iters": n_iters, "tenure": tenure, "n_restarts": n_restarts}


def solve_via_qubo(A, w, c, K, *, method="simulated_annealing",
                   encoding="slack", seed=0, alpha=1.0, beta=0.0, **kw) -> Result:
    """Build the QUBO, solve it, decode, and score natively.

    The decode-then-score-natively step is what makes a QUBO solver's result
    comparable to a native solver's: an infeasible QUBO optimum is reported as
    infeasible rather than silently accepted.
    """
    q = qb.build_max_coverage(A, w, c, K, encoding=encoding, alpha=alpha, beta=beta)
    fn = {"simulated_annealing": simulated_annealing, "tabu": tabu}[method]
    t0 = time.perf_counter()
    z, energy, meta = fn(q, seed=seed, **kw)
    dt = (time.perf_counter() - t0) * 1e3
    x = q.varmap.x_of(z)
    res = _summarise(A, w, c, x, K, method, dt, alpha=alpha, beta=beta, seed=seed,
                     formulation="qubo", encoding=encoding,
                     qubo_energy=energy, **meta)
    res.n_vars = q.n_vars
    return res
