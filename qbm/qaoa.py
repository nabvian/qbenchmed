"""QAOA on validated QUBOs (spec1 sections 13-15, 43, 46; spec3 sections 9-11).

Three decisions here shape what the benchmark can honestly claim.

*The cost layer is applied as an exact diagonal phase.*  The cost Hamiltonian is
diagonal in the computational basis, so ``exp(-i gamma H_C)`` is an elementwise
phase multiplication -- no gate sequence, no Trotter error, and cost O(2^n)
instead of O(2^n * n^2).  The simulation is therefore exact, but the resource
counters must not report the shortcut: :func:`ising_resource_cost` charges the
RZZ/RZ decomposition that hardware would run.  Reporting simulator convenience
as a circuit cost would flatter QAOA in exactly the dimension spec1 section 43
exists to measure.

*Expectation and sampling are reported separately.*  The noiseless expectation
value is available in closed form, and it is the objective the parameter
optimizer minimises.  It is not, however, what a real device would give you, and
a good expectation can coexist with a low probability of ever sampling the
optimum.  :class:`QaoaOutcome` carries both, plus the feasibility statistics of
the sampled distribution (spec3 section 11).

*Depth is measured, not assumed to help.*  ``p`` is a parameter of the
experiment, and the ladder p=1,2,3 is run and reported, including when deeper
circuits do worse -- which reparameterisation and optimizer stalls both cause.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field

import numpy as np
from scipy.optimize import minimize

from .qubo import Qubo
from .simulator import StateVector

ALGORITHM_VERSION = "qaoa-1.0.0"

#: Derivative-free optimizers considered reliable enough for the MVP.  SPSA is
#: deliberately absent: it is valuable for noisy/hardware objectives, and adding
#: it against an exact noiseless expectation would only add tuning burden.
OPTIMIZERS = ("COBYLA", "Nelder-Mead", "Powell")


def ising_resource_cost(h: np.ndarray, J: np.ndarray, p: int,
                        *, count_state_prep: bool = True) -> dict:
    """Gate counts for a depth-``p`` QAOA circuit on the given Ising model.

    Counted against the standard decomposition -- RZZ as CNOT-RZ-CNOT, one RZ
    per nonzero field, one RX per qubit per mixer layer -- so the numbers
    describe hardware, not this simulator.  ``count_state_prep`` charges the
    initial Hadamard layer, which is a reporting choice rather than a physical
    one and is therefore explicit.
    """
    n = len(h)
    n_couplings = int(np.count_nonzero(np.triu(J, 1)))
    n_fields = int(np.count_nonzero(h))

    per_layer_2q = 2 * n_couplings                       # 2 CNOTs per RZZ
    per_layer_1q = n_couplings + n_fields + n            # RZ in RZZ, RZ, RX
    gate_count = p * (per_layer_2q + per_layer_1q) + (n if count_state_prep else 0)

    # depth: each RZZ contributes 3 sequential layers on its qubits; couplings on
    # disjoint qubits parallelise, so this is an upper bound reported as such.
    depth = p * (3 * n_couplings + 1 + 1) + (1 if count_state_prep else 0)
    return {"n_qubits": n, "gate_count": gate_count,
            "two_qubit_gate_count": p * per_layer_2q,
            "depth_upper_bound": depth, "n_couplings": n_couplings,
            "n_fields": n_fields, "parameter_count": 2 * p}


@dataclass
class OptimizerRecord:
    """Everything spec3 section 10 requires about a parameter-optimizer run."""

    optimizer: str
    initial_parameters: list[float]
    final_parameters: list[float]
    iterations: int
    function_evaluations: int
    stopping_reason: str
    best_expectation: float
    seed: int | None
    runtime_ms: float

    def as_dict(self) -> dict:
        return {k: v for k, v in self.__dict__.items()}


@dataclass
class QaoaOutcome:
    """Expectation-level and sample-level quality, kept distinct (spec3 s11)."""

    depth: int
    final_expectation: float
    best_sampled_energy: float
    best_sampled_state: list[int]
    best_sample_probability: float
    optimum_probability: float
    near_optimum_probability: float
    feasible_probability: float
    shots: int
    resources: dict
    optimizer: OptimizerRecord
    runtime_ms: float
    extra: dict = field(default_factory=dict)

    def as_dict(self) -> dict:
        d = dict(self.__dict__)
        d["optimizer"] = self.optimizer.as_dict()
        return d


class Qaoa:
    """Depth-``p`` QAOA for a :class:`~qbm.qubo.Qubo`.

    The QUBO is converted once to its Ising form for resource accounting, and
    its diagonal is cached for the cost layer.  Both refer to the same
    formulation, so a QAOA result is comparable with a classical solver run on
    the identical QUBO -- the comparison spec1 section 16.4 asks for.
    """

    def __init__(self, qubo: Qubo, *, memory_limit_bytes: int | None = None,
                 count_state_prep: bool = True):
        self.qubo = qubo
        self.n = qubo.n_vars
        self.memory_limit_bytes = memory_limit_bytes
        self.count_state_prep = count_state_prep

        # probe the memory guard before any expensive setup so an infeasible
        # instance fails with SimulationResourceError, not a MemoryError
        StateVector(self.n, memory_limit_bytes=memory_limit_bytes)

        self.h, self.J, self.ising_const = qubo.to_ising()
        self.diag = qubo.diagonal_energies()          # cost Hamiltonian diagonal

    # -- circuit -------------------------------------------------------------
    def state(self, gammas: np.ndarray, betas: np.ndarray) -> StateVector:
        """Prepare |psi(gamma, beta)>.

        The cost layer is ``exp(-i gamma_k E(x))`` applied elementwise; the
        mixer is RX(2 beta_k) on every qubit, matching the standard
        ``H_B = sum_i X_i`` convention.
        """
        sv = StateVector.plus_state(self.n,
                                    memory_limit_bytes=self.memory_limit_bytes)
        phase_shape = (2,) * self.n
        for gamma, beta in zip(gammas, betas):
            sv.psi = sv.psi * np.exp(-1j * gamma * self.diag).reshape(phase_shape)
            sv.rx_all(float(beta))
        return sv

    def expectation(self, params: np.ndarray) -> float:
        """Exact <psi| H_C |psi>, the optimizer's objective.

        Exact rather than shot-estimated: sampling noise would make the
        derivative-free optimizers chase variance, and the honest place to
        introduce shot noise is the sampling stage, which is reported separately.
        """
        p = len(params) // 2
        sv = self.state(params[:p], params[p:])
        return sv.expectation_diagonal(self.diag)

    # -- parameter optimization ---------------------------------------------
    def optimize(self, p: int, *, optimizer: str = "COBYLA", seed: int | None = None,
                 maxiter: int = 600, n_restarts: int = 1) -> OptimizerRecord:
        """Optimize (gamma, beta) at depth ``p``.

        Multiple random restarts are supported because the QAOA landscape is
        non-convex and a single start conflates "QAOA is weak here" with "the
        classical outer loop got stuck" -- two very different findings.  The
        restart count is recorded so the classical effort spent is visible.
        """
        if optimizer not in OPTIMIZERS:
            raise ValueError(f"unknown optimizer {optimizer!r}; use one of {OPTIMIZERS}")
        rng = np.random.default_rng(seed)
        t0 = time.perf_counter()

        best = None
        total_nfev = total_nit = 0
        for _ in range(max(1, n_restarts)):
            # gamma in [0, pi), beta in [0, pi/2): one period of each layer
            x0 = np.concatenate([rng.uniform(0, np.pi, p),
                                 rng.uniform(0, np.pi / 2, p)])
            res = minimize(self.expectation, x0, method=optimizer,
                           options={"maxiter": maxiter})
            total_nfev += int(getattr(res, "nfev", 0) or 0)
            total_nit += int(getattr(res, "nit", 0) or 0)
            if best is None or res.fun < best.fun:
                best, best_x0 = res, x0

        runtime_ms = (time.perf_counter() - t0) * 1e3
        return OptimizerRecord(
            optimizer=optimizer,
            initial_parameters=[float(v) for v in best_x0],
            final_parameters=[float(v) for v in best.x],
            iterations=total_nit,
            function_evaluations=total_nfev,
            stopping_reason=str(getattr(best, "message", "") or "converged"),
            best_expectation=float(best.fun),
            seed=seed,
            runtime_ms=runtime_ms)

    # -- full run ------------------------------------------------------------
    def run(self, p: int, *, shots: int = 10_000, optimizer: str = "COBYLA",
            seed: int | None = None, maxiter: int = 600, n_restarts: int = 1,
            optimum_energy: float | None = None, epsilon: float = 1e-6,
            feasible_fn=None) -> QaoaOutcome:
        """Optimize parameters, then sample, reporting both quality levels.

        ``optimum_energy`` should be the certified optimum of *this QUBO* when
        known, so ``optimum_probability`` is measured against ground truth rather
        than against the best value QAOA happened to find.
        """
        t0 = time.perf_counter()
        rec = self.optimize(p, optimizer=optimizer, seed=seed, maxiter=maxiter,
                            n_restarts=n_restarts)
        params = np.asarray(rec.final_parameters)
        sv = self.state(params[:p], params[p:])

        states, counts = sv.measure(shots=shots, seed=seed)
        energies = self.qubo.energies(states.astype(bool))
        probs = counts / counts.sum()

        k_best = int(np.argmin(energies))
        best_energy = float(energies[k_best])

        ref = best_energy if optimum_energy is None else float(optimum_energy)
        # exact probabilities from the state, not the sample, for optimum mass:
        # sampling gives a noisy estimate of a quantity we can compute exactly
        all_p = sv.probabilities().reshape(-1)
        opt_mask = self.diag <= ref + epsilon
        near_mask = self.diag <= ref + max(epsilon, abs(ref) * 0.01)
        optimum_probability = float(all_p[opt_mask].sum())
        near_probability = float(all_p[near_mask].sum())

        if feasible_fn is None:
            feasible_probability = float("nan")
        else:
            flags = np.array([bool(feasible_fn(s.astype(bool))) for s in states])
            feasible_probability = float(probs[flags].sum())

        resources = ising_resource_cost(self.h, self.J, p,
                                        count_state_prep=self.count_state_prep)
        resources["shots"] = shots
        resources["optimizer_iterations"] = rec.iterations
        resources["simulator_gate_count"] = sv.resources.gate_count

        return QaoaOutcome(
            depth=p,
            final_expectation=rec.best_expectation,
            best_sampled_energy=best_energy,
            best_sampled_state=[int(v) for v in states[k_best]],
            best_sample_probability=float(probs[k_best]),
            optimum_probability=optimum_probability,
            near_optimum_probability=near_probability,
            feasible_probability=feasible_probability,
            shots=shots,
            resources=resources,
            optimizer=rec,
            runtime_ms=(time.perf_counter() - t0) * 1e3,
            extra={"ising_const": self.ising_const,
                   "reference_energy": ref,
                   "n_restarts": n_restarts,
                   "algorithm_version": ALGORITHM_VERSION})
