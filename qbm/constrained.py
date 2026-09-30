"""Constrained-mixer QAOA: the experiment the penalty-landscape measurement points at.

The head-to-head result (see ``reproducibility/REPORT.md`` and ``RESULTS.md``)
is a property of the *encoding*, not of any solver.  The budget ``sum_i x_i <=
K`` is imposed as a penalty term, and that term runs 85-374x larger than the
coverage signal, leaving under 1% of the QUBO spectrum holding anything useful.
QAOA then spends its variational capacity climbing out of the penalty bulk
rather than discriminating among good panels.

The standard answer -- Hadfield's Quantum Alternating Operator Ansatz -- removes
the penalty entirely.  Instead of penalising infeasible strings, you never
leave the feasible set:

* the initial state is the Dicke state ``|D^n_K>``, the equal superposition of
  every string with exactly ``K`` ones;
* the mixer is the ``XY`` ring mixer ``sum_edges (X_i X_j + Y_i Y_j)``, every
  term of which commutes with total Hamming weight;
* the cost layer is the coverage objective **with no budget term and no slack
  qubits**.

Weight is conserved at every step, so 100% of the sampled distribution is
feasible by construction, and the circuit runs on ``n`` input qubits rather than
``n + ceil(log2(K+1))``.  "at most K" collapses to "exactly K" without loss
because coverage is monotone non-decreasing in inputs when no per-input cost is
charged -- adding an input can only cover more -- and :func:`optimum_at_weight`
checks that equality rather than assuming it.

This module is a proof of concept.  It is deliberately small, it runs on the
same generated instances as the head-to-head, and it reports the one number the
measurement predicts should move: the probability of sampling the optimum at a
fixed depth.  The ring mixer is applied as a two-colour (even/odd edge) split,
which is a first-order Trotter step and exactly what hardware would run; the
Dicke preparation is constructed directly in the simulator, and the CNOT count
of its real gate-level circuit is reported rather than hidden.
"""
from __future__ import annotations

import time
from dataclasses import dataclass, field

import numpy as np
from scipy.optimize import minimize

from .simulator import StateVector

ALGORITHM_VERSION = "constrained-qaoa-0.1.0"


def coverage_diagonal(A: np.ndarray, w: np.ndarray,
                      arms: list[tuple[int, tuple[int, ...]]] | None = None) -> np.ndarray:
    """Minimisation-sense coverage energy of every string over the n input qubits.

    ``E(x) = - sum_j w_j * [outcome j is covered by x]``, with "covered" meaning
    exactly what :func:`qbm.qubo.coverage_mask` means -- the one definition every
    solver uses.  Without arms, an outcome is covered when any contributing input
    is selected.  With arms, it is covered when *every* input of at least one of
    its arms is selected; an empty arm covers unconditionally.

    Because the cost layer is applied as a diagonal phase over the inputs, a
    conjunctive rule costs no extra qubits here.  The penalty encoding needs an
    auxiliary variable per outcome plus slack to express the same thing.

    Returned as a ``(2,) * n`` tensor so it drops straight into the simulator's
    diagonal-phase cost layer.
    """
    A = np.asarray(A, dtype=np.float64)
    w = np.asarray(w, dtype=np.float64)
    n_in, n_out = A.shape
    idx = np.arange(2 ** n_in, dtype=np.int64)
    all_bits = np.stack(np.unravel_index(idx, (2,) * n_in), axis=-1).astype(bool)
    covered = np.zeros((2 ** n_in, n_out), dtype=bool)
    if arms is None:
        for j in range(n_out):
            inputs = np.flatnonzero(A[:, j])
            if inputs.size:
                covered[:, j] = all_bits[:, inputs].any(axis=1)
    else:
        for j, members in arms:
            mem = list(members)
            covered[:, j] |= all_bits[:, mem].all(axis=1) if mem else True
    energy = -(covered.astype(np.float64) @ w)
    return energy.reshape((2,) * n_in)


def optimum_at_weight(diag: np.ndarray, k: int) -> tuple[float, float]:
    """Best coverage energy at exactly ``k`` inputs, and over *all* selections of <= k.

    The two are equal when coverage is monotone in inputs, which it is here.
    Returning both lets the caller assert that the exactly-``k`` subspace has
    not thrown away the "at most k" optimum -- the one correctness risk the
    subspace restriction introduces.
    """
    flat = diag.reshape(-1)
    n = diag.ndim
    if not 0 <= k <= n:
        raise ValueError(f"weight {k} out of range for {n} inputs")
    idx = np.arange(flat.size, dtype=np.int64)
    weights = np.stack(np.unravel_index(idx, (2,) * n), axis=-1).sum(axis=1)
    at_k = flat[weights == k]
    le_k = flat[weights <= k]
    return float(at_k.min()), float(le_k.min())


def dicke_state(n: int, k: int, **kw) -> StateVector:
    """``|D^n_k>``: equal superposition of all strings with exactly ``k`` ones.

    Built directly, the way :meth:`StateVector.plus_state` builds ``|+>^n``.
    On hardware this is a state-preparation circuit of O(nk) depth; that cost is
    reported by :func:`dicke_prep_cost` rather than folded into the mixer count.
    """
    if not 0 <= k <= n:
        raise ValueError(f"weight {k} out of range for {n} qubits")
    sv = StateVector(n, **kw)
    idx = np.arange(2 ** n, dtype=np.int64)
    bits = np.stack(np.unravel_index(idx, (2,) * n), axis=-1)
    mask = bits.sum(axis=1) == k
    amp = np.zeros(2 ** n, dtype=np.complex128)
    amp[mask] = 1.0 / np.sqrt(mask.sum())
    sv.psi = amp.reshape((2,) * n)
    return sv


def dicke_prep_cost(n: int, k: int) -> dict:
    """Two-qubit cost of preparing ``|D^n_k>``, counted from the real circuit.

    The count is the number of CNOTs in :func:`qbm.interop.dicke_gates`, the
    Baertschi-Eidenbenz (2019) preparation this project actually emits, before
    any transpiler optimisation.  That makes it an upper bound: a transpiler
    removes roughly 15-20% of them.

    An earlier revision used a ``5nk`` formula and described it as a
    conservative overestimate.  It was not: from n = 10 upward the real circuit
    needs more than ``5nk``.  Counting the circuit replaces the formula so the
    number cannot drift from the thing it describes.
    """
    from .interop import dicke_gates          # local: interop imports this module's peers
    gates = dicke_gates(n, k)
    return {"prep_two_qubit_gates": sum(1 for g in gates if g[0] == "cx"),
            "basis": "CNOTs in the emitted Baertschi-Eidenbenz circuit, untranspiled (upper bound)"}


#: CNOTs per two-qubit interaction on a device whose only entangling gate is CX.
#: An RZZ is CNOT-RZ-CNOT; an XY(theta) rotation synthesises to two CNOTs as
#: well (Qiskit's transpiler confirms both; see tests/test_interop.py).  Charging
#: both the same keeps the two ansatzes on one honest scale.
CNOT_PER_RZZ = 2
CNOT_PER_XY = 2


def ising_terms(diag: np.ndarray, tol: float = 1e-9) -> dict[int, int]:
    """Number of nonzero Z-terms of each order in the cost Hamiltonian ``diag``.

    A Walsh-Hadamard transform of the diagonal gives the exact expansion
    ``E = sum_S c_S prod_{i in S} Z_i``; this counts the ``S`` with ``c_S != 0``
    by ``|S|``.  It is exact for any coverage rule -- disjunctive, conjunctive,
    or a mix -- which a count of pairs in ``A`` is not: an OR of AND-arms expands
    into three- and four-body terms that the incidence matrix does not show.
    """
    f = np.asarray(diag, dtype=np.float64).copy()
    for axis in range(f.ndim):                 # bit 0 <-> z=+1, bit 1 <-> z=-1
        a = np.take(f, 0, axis=axis)
        b = np.take(f, 1, axis=axis)
        f = np.stack([(a + b) / 2, (a - b) / 2], axis=axis)
    n = f.ndim
    idx = np.arange(2 ** n)
    order = np.stack(np.unravel_index(idx, (2,) * n), axis=-1).sum(axis=1)
    nonzero = np.abs(f.reshape(-1)) > tol
    counts: dict[int, int] = {}
    for m in order[nonzero]:
        counts[int(m)] = counts.get(int(m), 0) + 1
    counts.pop(0, None)                         # the constant is a global phase
    return dict(sorted(counts.items()))


def constrained_resource_cost(diag: np.ndarray, K: int, p: int) -> dict:
    """Two-qubit gate count for depth-``p`` constrained QAOA, charged like the penalty side.

    Cost layer: the exact Z-term expansion of the coverage Hamiltonian
    (:func:`ising_terms`), each ``m``-body term charged ``2(m-1)`` CNOTs -- a CX
    ladder, an RZ, and the ladder undone.  For ``m = 2`` that is the familiar
    CNOT-RZ-CNOT, ``CNOT_PER_RZZ``.  Ladders are not shared between terms, so
    this is an upper bound; a compiler can do better on dense instances.

    Mixer: one XY gate per ring edge at ``CNOT_PER_XY`` CNOTs.

    Takes the cost diagonal itself (``coverage_diagonal(A, w, arms)``), so the
    count describes exactly the Hamiltonian the ansatz evolves under.

    The cost layer is not free just because the simulator applies it as a
    diagonal phase -- the same rule :func:`~qbm.qaoa.ising_resource_cost`
    applies to the penalty ansatz.
    """
    n = np.asarray(diag).ndim
    terms = ising_terms(diag)
    cost_2q = sum(count * 2 * (m - 1) for m, count in terms.items() if m >= 2)
    n_edges = n if n > 2 else max(n - 1, 0)
    mixer_2q = CNOT_PER_XY * n_edges
    prep = dicke_prep_cost(n, K)
    return {
        "n_qubits": n,
        "cost_layer_two_qubit_per_layer": cost_2q,
        "mixer_two_qubit_per_layer": mixer_2q,
        "two_qubit_gate_count": p * (cost_2q + mixer_2q),
        "two_qubit_gate_count_with_prep": p * (cost_2q + mixer_2q)
                                          + prep["prep_two_qubit_gates"],
        "ising_terms_by_order": terms,
        "n_coverage_couplings": terms.get(2, 0),
        "higher_order_terms": sum(c for m, c in terms.items() if m > 2),
        "mixer_edges": n_edges,
        "parameter_count": 2 * p,
        "dicke_prep": prep,
    }


def _xy_gate(theta: float) -> np.ndarray:
    """exp(-i theta (X0 X1 + Y0 Y1) / 2): a partial swap on the |01>,|10> block.

    Fixes |00> and |11>, so total Hamming weight is preserved exactly.  Basis
    order |q0 q1> = 00, 01, 10, 11.
    """
    c, s = np.cos(theta), np.sin(theta)
    return np.array([
        [1, 0, 0, 0],
        [0, c, -1j * s, 0],
        [0, -1j * s, c, 0],
        [0, 0, 0, 1],
    ], dtype=np.complex128)


def apply_ring_mixer(sv: StateVector, beta: float) -> StateVector:
    """The XY ring mixer at angle ``beta``, as a two-colour Trotter step.

    Ring edges split into even ``(0,1),(2,3),...`` and odd ``(1,2),(3,4),...``
    colours; within a colour the edges are disjoint so their XY gates commute
    and apply exactly.  Applying even then odd is one first-order Trotter step of
    ``exp(-i beta H_M)`` -- the standard hardware-realisable mixer, not an exact
    exponential of the full (non-commuting) ring.
    """
    n = sv.n
    if n < 2:
        return sv
    gate = _xy_gate(beta)
    for start in (0, 1):                       # even colour, then odd colour
        for i in range(start, n - 1, 2):
            sv.apply_2q(gate, i, i + 1)
    if n > 2:                                  # close the ring
        sv.apply_2q(gate, n - 1, 0)
    return sv


@dataclass
class ConstrainedOutcome:
    """What a constrained-mixer run delivers, alongside the penalty run it is compared to."""

    depth: int
    parameters: list[float]              # optimised (gammas..., betas...), replayable elsewhere
    final_expectation: float
    optimum_probability: float
    feasible_probability: float          # 1.0 by construction; measured, not assumed
    best_sampled_energy: float
    best_sampled_state: list[int]
    n_qubits: int
    mixer: str
    resources: dict
    optimizer_iterations: int
    parameter_init: str
    runtime_ms: float
    extra: dict = field(default_factory=dict)

    def as_dict(self) -> dict:
        return dict(self.__dict__)


class ConstrainedQaoa:
    """Depth-``p`` QAOA with an XY ring mixer on the exactly-``K`` subspace.

    Takes the coverage structure directly (``A``, ``w``, ``K``) rather than a
    penalised :class:`~qbm.qubo.Qubo`, because the entire point is that no
    penalty and no slack register exist here.
    """

    def __init__(self, A: np.ndarray, w: np.ndarray, K: int,
                 *, arms: list[tuple[int, tuple[int, ...]]] | None = None,
                 memory_limit_bytes: int | None = None):
        self.A = np.asarray(A, dtype=np.float64)
        self.w = np.asarray(w, dtype=np.float64)
        self.K = int(K)
        self.arms = arms
        self.n = self.A.shape[0]
        self.diag = coverage_diagonal(self.A, self.w, arms)
        self.memory_limit_bytes = memory_limit_bytes
        StateVector(self.n, memory_limit_bytes=memory_limit_bytes)   # probe memory

    def state(self, gammas: np.ndarray, betas: np.ndarray) -> StateVector:
        sv = dicke_state(self.n, self.K, memory_limit_bytes=self.memory_limit_bytes)
        for gamma, beta in zip(gammas, betas):
            # exp(-i gamma E) computed directly.  Raising a precomputed exp(-i E)
            # to the power gamma is NOT equivalent: a complex power takes the
            # principal log, which wraps each energy into (-pi, pi], and coverage
            # energies here run well outside that.  That exact mistake shipped
            # once and was caught only by comparing against the exported circuit.
            sv.psi = sv.psi * np.exp(-1j * gamma * self.diag)
            apply_ring_mixer(sv, float(beta))
        return sv

    def expectation(self, params: np.ndarray) -> float:
        p = len(params) // 2
        sv = self.state(params[:p], params[p:])
        return sv.expectation_diagonal(self.diag.reshape(-1))

    def _grid_seeds(self, p: int, resolution: int) -> list[np.ndarray]:
        """Coarse (gamma, beta) sweep, best-first.

        The coverage-only spectrum spans only ``sum_j w_j`` (tens of units, not
        thousands), so the landscape is smooth and a modest grid suffices -- the
        smoothness the penalty encoding destroyed is exactly what the subspace
        restores.  gamma period is 2*pi for the integer-weighted coverage
        spectrum; beta period is 2*pi for the XY pair rotation.
        """
        gammas = np.linspace(0.0, 2 * np.pi, resolution, endpoint=False)
        betas = np.linspace(0.0, np.pi, max(8, resolution // 4), endpoint=False)
        scored = []
        for g in gammas:
            for b in betas:
                params = np.concatenate([np.full(p, g), np.full(p, b)])
                scored.append((self.expectation(params), g, b))
        scored.sort(key=lambda t: t[0])
        return [np.concatenate([np.full(p, g), np.full(p, b)]) for _, g, b in scored]

    def run(self, p: int, *, shots: int = 10_000, optimizer: str = "COBYLA",
            seed: int | None = None, maxiter: int = 600, n_restarts: int = 1,
            grid_resolution: int = 48, optimum_energy: float | None = None,
            epsilon: float = 1e-6) -> ConstrainedOutcome:
        t0 = time.perf_counter()
        ranked = self._grid_seeds(p, grid_resolution)
        starts = ranked[:max(1, n_restarts)]
        best = None
        best_x0 = starts[0]
        total_nit = 0
        for x0 in starts:
            res = minimize(self.expectation, x0, method=optimizer,
                           options={"maxiter": maxiter})
            total_nit += int(getattr(res, "nit", 0) or 0)
            if best is None or res.fun < best.fun:
                best, best_x0 = res, x0

        params = np.asarray(best.x)
        sv = self.state(params[:p], params[p:])
        all_p = sv.probabilities().reshape(-1)

        ref = float(self.diag.min()) if optimum_energy is None else float(optimum_energy)
        flat_diag = self.diag.reshape(-1)
        opt_mask = flat_diag <= ref + epsilon
        optimum_probability = float(all_p[opt_mask].sum())

        # Feasibility is exactly-K weight; measured from the sampled distribution
        # so the "conserved by construction" claim is checked, not asserted.
        states, counts = sv.measure(shots=shots, seed=seed)
        probs = counts / counts.sum()
        weights = states.sum(axis=1)
        feasible_probability = float(probs[weights == self.K].sum())
        energies = np.array([self.diag[tuple(s)] for s in states])
        k_best = int(np.argmin(energies))

        resources = constrained_resource_cost(self.diag, self.K, p)
        resources["shots"] = shots
        return ConstrainedOutcome(
            depth=p,
            parameters=[float(v) for v in params],
            final_expectation=float(best.fun),
            optimum_probability=optimum_probability,
            feasible_probability=feasible_probability,
            best_sampled_energy=float(energies[k_best]),
            best_sampled_state=[int(v) for v in states[k_best]],
            n_qubits=self.n,
            mixer="xy_ring",
            resources=resources,
            optimizer_iterations=total_nit,
            parameter_init="grid",
            runtime_ms=(time.perf_counter() - t0) * 1e3,
            extra={"reference_energy": ref,
                   "n_restarts": n_restarts,
                   "algorithm_version": ALGORITHM_VERSION})
