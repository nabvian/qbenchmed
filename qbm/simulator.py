"""State-vector simulator for Q-BenchMed (spec sections 36-37, 43).

Design notes that matter for the benchmark rather than for the simulation:

*Gate application is tensor-reshape, not matrix construction.*  Building the
full 2^n x 2^n operator would cap the simulator near 13 qubits.  Reshaping the
state to expose the target axis and contracting a 2x2 (or 4x4) block against it
keeps memory at the state vector itself, which is what sets the ~30-qubit wall
quoted throughout the project.

*Qubit ordering is tensor-axis ordering, and it is derived, never assumed.*
Axis ``q`` of the state tensor is qubit ``q`` is variable ``q`` of the QUBO, so
``psi[b_0, b_1, ..., b_{n-1}]`` is the amplitude of the assignment
``(b_0, ..., b_{n-1})`` directly.  Because ``reshape(-1)`` flattens in C order,
this makes qubit 0 the *most* significant bit of the flat basis index -- the
opposite of the Qiskit convention, so it is stated rather than left implicit.

An inconsistent convention here is the classic silent QAOA bug: circuits build
correctly, energies look plausible, and the decoded solution names the wrong
inputs.  Two defences: ``index_to_bits`` uses ``np.unravel_index`` against the
state's own shape rather than hand-rolled shift arithmetic, so it cannot drift
out of step with ``reshape``; and the mapping is asserted in the test suite
against explicit Kronecker products.  An earlier revision of this file
documented little-endian while implementing big-endian, and the test caught it.

*Resource counting is built in, not bolted on.*  Every gate application
increments the counters spec section 43 requires (gate count, two-qubit gate
count, depth), because a QAOA result without its resource cost is not
interpretable.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np

# --------------------------------------------------------------------------
# gate definitions -- the single source of truth, tested against known matrices
# --------------------------------------------------------------------------
_ISQRT2 = 1.0 / np.sqrt(2.0)

I2 = np.eye(2, dtype=np.complex128)
X = np.array([[0, 1], [1, 0]], dtype=np.complex128)
Y = np.array([[0, -1j], [1j, 0]], dtype=np.complex128)
Z = np.array([[1, 0], [0, -1]], dtype=np.complex128)
H = _ISQRT2 * np.array([[1, 1], [1, -1]], dtype=np.complex128)
S = np.array([[1, 0], [0, 1j]], dtype=np.complex128)
T = np.array([[1, 0], [0, np.exp(1j * np.pi / 4)]], dtype=np.complex128)

CNOT = np.array([[1, 0, 0, 0],
                 [0, 1, 0, 0],
                 [0, 0, 0, 1],
                 [0, 0, 1, 0]], dtype=np.complex128)
CZ = np.diag([1, 1, 1, -1]).astype(np.complex128)


def rx(theta: float) -> np.ndarray:
    c, s = np.cos(theta / 2), np.sin(theta / 2)
    return np.array([[c, -1j * s], [-1j * s, c]], dtype=np.complex128)


def ry(theta: float) -> np.ndarray:
    c, s = np.cos(theta / 2), np.sin(theta / 2)
    return np.array([[c, -s], [s, c]], dtype=np.complex128)


def rz(theta: float) -> np.ndarray:
    e = np.exp(-1j * theta / 2)
    return np.array([[e, 0], [0, np.conj(e)]], dtype=np.complex128)


def phase(theta: float) -> np.ndarray:
    return np.array([[1, 0], [0, np.exp(1j * theta)]], dtype=np.complex128)


GATES_1Q = {"I": I2, "X": X, "Y": Y, "Z": Z, "H": H, "S": S, "T": T}
GATES_2Q = {"CNOT": CNOT, "CZ": CZ}


# --------------------------------------------------------------------------
@dataclass
class Resources:
    """Quantum resource metrics (spec section 43)."""

    n_qubits: int = 0
    gate_count: int = 0
    two_qubit_gate_count: int = 0
    depth: int = 0
    _frontier: dict = field(default_factory=dict, repr=False)

    def record(self, qubits: tuple[int, ...]) -> None:
        self.gate_count += 1
        if len(qubits) == 2:
            self.two_qubit_gate_count += 1
        # depth = longest chain of gates sharing a qubit
        layer = max((self._frontier.get(q, 0) for q in qubits), default=0) + 1
        for q in qubits:
            self._frontier[q] = layer
        self.depth = max(self.depth, layer)

    def as_dict(self) -> dict:
        return {"n_qubits": self.n_qubits, "gate_count": self.gate_count,
                "two_qubit_gate_count": self.two_qubit_gate_count,
                "depth": self.depth}


class SimulationResourceError(MemoryError):
    """Raised before any allocation when a state vector cannot fit the budget.

    Carries the numbers needed to report the simulation wall in a paper rather
    than just failing: an experiment that hits this should record *why* the
    instance was out of reach, not merely that it was.
    """

    def __init__(self, requested_qubits: int, memory_limit_bytes: int):
        self.requested_qubits = int(requested_qubits)
        self.estimated_statevector_bytes = 16 * 2 ** int(requested_qubits)
        self.configured_memory_limit = int(memory_limit_bytes)
        self.recommended_max_qubits = int(
            np.floor(np.log2(max(memory_limit_bytes, 16) / 16)))
        super().__init__(
            f"{self.requested_qubits} qubits needs "
            f"{self.estimated_statevector_bytes / 2**30:.2f} GiB of complex128 "
            f"state, limit is {self.configured_memory_limit / 2**30:.2f} GiB "
            f"(recommended max {self.recommended_max_qubits} qubits)")

    def as_dict(self) -> dict:
        return {"requested_qubits": self.requested_qubits,
                "estimated_statevector_bytes": self.estimated_statevector_bytes,
                "configured_memory_limit": self.configured_memory_limit,
                "recommended_max_qubits": self.recommended_max_qubits}


class NumericalError(RuntimeError):
    """Raised when amplitudes go non-finite or the norm drifts materially.

    Silent numerical corruption in a QAOA run produces plausible-looking
    energies from a meaningless state, so this is a hard failure rather than a
    warning.
    """


def max_simulable_qubits(available_gib: float, fraction: float = 0.35) -> int:
    """Largest n whose complex128 state vector fits in the given budget.

    Used to justify the simulation wall quoted in the report rather than
    asserting a round number.
    """
    budget_bytes = fraction * available_gib * 2 ** 30
    return int(np.floor(np.log2(budget_bytes / 16)))


# --------------------------------------------------------------------------
class StateVector:
    """Little-endian state-vector simulator.

    The state is held as an ``(2,) * n`` complex128 tensor; axis ``k`` is qubit
    ``k``.  This makes single- and two-qubit gate application a small tensordot
    with no large intermediate operator.
    """

    #: Default budget: 16 GiB, i.e. 30 qubits of complex128 state.
    DEFAULT_MEMORY_LIMIT = 16 * 2 ** 30

    def __init__(self, n_qubits: int, *, memory_limit_bytes: int | None = None):
        n_qubits = int(n_qubits)
        if n_qubits < 1:
            raise ValueError(f"need at least one qubit, got {n_qubits}")
        limit = (self.DEFAULT_MEMORY_LIMIT if memory_limit_bytes is None
                 else int(memory_limit_bytes))
        # estimate before allocating -- never attempt an impossible allocation
        if 16 * 2 ** n_qubits > limit:
            raise SimulationResourceError(n_qubits, limit)
        self.memory_limit_bytes = limit
        self.n = n_qubits
        self.psi = np.zeros((2,) * self.n, dtype=np.complex128)
        self.psi[(0,) * self.n] = 1.0
        self.resources = Resources(n_qubits=self.n)

    # -- construction helpers ------------------------------------------------
    @classmethod
    def plus_state(cls, n_qubits: int, **kw) -> "StateVector":
        """|+>^n, the QAOA initial state, built directly rather than via n H gates.

        Building it directly keeps the resource counters honest: the H layer is
        state preparation, and whether to charge it to the circuit is a
        reporting decision, so ``apply_h_layer`` exists for when it should be.
        """
        sv = cls(n_qubits, **kw)
        sv.psi = np.full((2,) * n_qubits, 2.0 ** (-n_qubits / 2), dtype=np.complex128)
        return sv

    def apply_h_layer(self) -> "StateVector":
        for q in range(self.n):
            self.apply_1q(H, q)
        return self

    # -- gate application ----------------------------------------------------
    def apply_1q(self, U: np.ndarray, q: int, *, count: bool = True) -> "StateVector":
        """Apply a 2x2 operator to qubit ``q``.

        ``count=False`` applies the operator without charging the resource
        counters.  It exists for noise injection: a stochastic error is not a
        gate the algorithm asked for, and counting it would inflate the reported
        circuit cost of a *noisy* run above the identical ideal run, making
        depth-vs-noise comparisons meaningless.  Algorithm gates must always be
        counted, so the default is ``True``.
        """
        if not 0 <= q < self.n:
            raise IndexError(f"qubit {q} out of range for {self.n} qubits")
        self.psi = np.moveaxis(np.tensordot(U, self.psi, axes=([1], [q])), 0, q)
        if count:
            self.resources.record((q,))
        return self

    def apply_2q(self, U: np.ndarray, q0: int, q1: int) -> "StateVector":
        """Apply a 4x4 gate, with ``q0`` the more significant index of U.

        U is indexed as |q0 q1>, matching the textbook CNOT matrix where the
        first listed qubit is the control.
        """
        if q0 == q1:
            raise ValueError("two-qubit gate needs distinct qubits")
        for q in (q0, q1):
            if not 0 <= q < self.n:
                raise IndexError(f"qubit {q} out of range for {self.n} qubits")
        U4 = U.reshape(2, 2, 2, 2)
        self.psi = np.moveaxis(
            np.tensordot(U4, self.psi, axes=([2, 3], [q0, q1])), [0, 1], [q0, q1])
        self.resources.record((q0, q1))
        return self

    def cnot(self, control: int, target: int) -> "StateVector":
        return self.apply_2q(CNOT, control, target)

    def cz(self, q0: int, q1: int) -> "StateVector":
        return self.apply_2q(CZ, q0, q1)

    def rzz(self, theta: float, q0: int, q1: int) -> "StateVector":
        """exp(-i theta Z_q0 Z_q1 / 2), applied as a diagonal phase.

        The QAOA cost layer is made almost entirely of these.  Applying the
        phase directly rather than as CNOT-RZ-CNOT saves two two-qubit gates per
        term in the *simulation*; the resource counter still charges the
        decomposed cost so reported gate counts reflect what hardware would run.
        """
        # diagonal in the computational basis: phase depends on parity
        shape = [1] * self.n
        shape[q0] = 2
        z0 = np.array([1.0, -1.0]).reshape(shape)
        shape2 = [1] * self.n
        shape2[q1] = 2
        z1 = np.array([1.0, -1.0]).reshape(shape2)
        self.psi = self.psi * np.exp(-0.5j * theta * z0 * z1)
        # charge the hardware-equivalent decomposition: CNOT, RZ, CNOT
        self.resources.record((q0, q1))
        self.resources.record((q1,))
        self.resources.record((q0, q1))
        return self

    def rz_all(self, thetas: np.ndarray) -> "StateVector":
        """Apply RZ(theta_q) to every qubit as one diagonal multiply."""
        thetas = np.asarray(thetas, dtype=float)
        if thetas.shape != (self.n,):
            raise ValueError(f"expected {self.n} angles, got {thetas.shape}")
        phase_tensor = np.ones((2,) * self.n, dtype=np.complex128)
        for q, th in enumerate(thetas):
            if th == 0.0:
                continue
            shape = [1] * self.n
            shape[q] = 2
            z = np.array([1.0, -1.0]).reshape(shape)
            phase_tensor = phase_tensor * np.exp(-0.5j * th * z)
            self.resources.record((q,))
        self.psi = self.psi * phase_tensor
        return self

    def rx_all(self, beta: float) -> "StateVector":
        """The QAOA mixer: RX(2 beta) on every qubit."""
        U = rx(2.0 * beta)
        for q in range(self.n):
            self.apply_1q(U, q)
        return self

    # -- readout -------------------------------------------------------------
    @property
    def vector(self) -> np.ndarray:
        """Flat amplitude array, little-endian basis ordering."""
        return self.psi.reshape(-1)

    #: Norm drift beyond this is a bug, not accumulated float error.
    NORM_TOLERANCE = 1e-8

    def check_numerics(self) -> "StateVector":
        """Fail loudly on NaN/Inf amplitudes or material norm drift."""
        if not np.all(np.isfinite(self.psi)):
            n_bad = int(np.sum(~np.isfinite(self.psi)))
            raise NumericalError(
                f"{n_bad} non-finite amplitude(s) in the state vector")
        total = float(np.sum(np.abs(self.psi) ** 2))
        if abs(total - 1.0) > self.NORM_TOLERANCE:
            raise NumericalError(
                f"state norm {total:.12g} deviates from 1 by "
                f"{abs(total - 1.0):.2e} (tolerance {self.NORM_TOLERANCE:.0e})")
        return self

    def probabilities(self) -> np.ndarray:
        self.check_numerics()
        p = np.abs(self.vector) ** 2
        # rescale only against residual float error, never against a real bug --
        # check_numerics has already refused anything larger
        return p / p.sum()

    def norm(self) -> float:
        return float(np.linalg.norm(self.vector))

    def index_to_bits(self, idx: np.ndarray | int) -> np.ndarray:
        """Flat basis index -> bit array in variable order (qubit 0 first).

        Uses ``unravel_index`` against the state's own shape, which is by
        construction the inverse of the ``reshape(-1)`` in :attr:`vector`.  Shift
        arithmetic would work too but can silently disagree with the flattening
        order, which is precisely the bug this project cannot afford.
        """
        idx = np.atleast_1d(np.asarray(idx, dtype=np.int64))
        return np.stack(np.unravel_index(idx, (2,) * self.n), axis=-1).astype(np.uint8)

    def bits_to_index(self, bits: np.ndarray) -> np.ndarray:
        """Inverse of :meth:`index_to_bits`, same derivation."""
        bits = np.atleast_2d(np.asarray(bits, dtype=np.int64))
        return np.ravel_multi_index(tuple(bits.T), (2,) * self.n)

    def measure(self, shots: int, seed: int | None = None) -> tuple[np.ndarray, np.ndarray]:
        """Sample the state.  Returns (unique_bitstrings, counts).

        Sampling unique outcomes with counts rather than a shots-long list keeps
        memory flat in the number of shots, which matters at 10k+ shots.
        """
        rng = np.random.default_rng(seed)
        p = self.probabilities()
        draws = rng.multinomial(shots, p)
        idx = np.flatnonzero(draws)
        return self.index_to_bits(idx), draws[idx]

    def expectation_diagonal(self, diag: np.ndarray) -> float:
        """<psi| D |psi> for a diagonal observable given as its diagonal.

        The QAOA cost Hamiltonian is diagonal in the computational basis, so the
        exact expectation value costs one dot product -- no sampling noise.  This
        is what makes noiseless QAOA parameter optimisation tractable here, and
        it is reported separately from sampled estimates (spec section 46).
        """
        p = self.probabilities()
        if diag.shape != p.shape:
            raise ValueError(f"diagonal shape {diag.shape} != state {p.shape}")
        return float(p @ diag)
