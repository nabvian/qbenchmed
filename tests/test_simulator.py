"""State-vector simulator validation (spec sections 36-38).

Every check compares against a value computed *independently* of the simulator:
a hand-written matrix, an explicit Kronecker product, or an analytically known
state.  Nothing here lets the simulator define its own correctness.
"""

from __future__ import annotations

import itertools

import numpy as np
import pytest

from qbm import simulator as sim


# --------------------------------------------------------------------------
# section 36 -- gate matrices against known values
# --------------------------------------------------------------------------
def test_pauli_algebra():
    assert np.allclose(sim.X @ sim.X, np.eye(2))
    assert np.allclose(sim.Y @ sim.Y, np.eye(2))
    assert np.allclose(sim.Z @ sim.Z, np.eye(2))
    # XY = iZ, YZ = iX, ZX = iY
    assert np.allclose(sim.X @ sim.Y, 1j * sim.Z)
    assert np.allclose(sim.Y @ sim.Z, 1j * sim.X)
    assert np.allclose(sim.Z @ sim.X, 1j * sim.Y)
    # anticommutation
    assert np.allclose(sim.X @ sim.Y + sim.Y @ sim.X, np.zeros((2, 2)))


@pytest.mark.parametrize("name", list(sim.GATES_1Q) + list(sim.GATES_2Q))
def test_gates_are_unitary(name):
    U = {**sim.GATES_1Q, **sim.GATES_2Q}[name]
    assert np.allclose(U.conj().T @ U, np.eye(U.shape[0])), f"{name} not unitary"


def test_hadamard_known_values():
    assert np.allclose(sim.H @ sim.H, np.eye(2))
    assert np.allclose(sim.H, sim.H.conj().T)                 # Hermitian
    assert np.allclose(sim.H @ np.array([1, 0]), [2 ** -0.5, 2 ** -0.5])
    assert np.allclose(sim.H @ np.array([0, 1]), [2 ** -0.5, -2 ** -0.5])
    # H X H = Z
    assert np.allclose(sim.H @ sim.X @ sim.H, sim.Z)


def test_s_and_t_gate_relations():
    assert np.allclose(sim.S @ sim.S, sim.Z)
    assert np.allclose(sim.T @ sim.T, sim.S)


@pytest.mark.parametrize("theta", [0.0, 0.3, np.pi / 2, np.pi, 2.4, 2 * np.pi])
def test_rotation_generators(theta):
    """R_a(theta) = cos(theta/2) I - i sin(theta/2) a, verified per axis."""
    c, s = np.cos(theta / 2), np.sin(theta / 2)
    assert np.allclose(sim.rx(theta), c * np.eye(2) - 1j * s * sim.X)
    assert np.allclose(sim.ry(theta), c * np.eye(2) - 1j * s * sim.Y)
    assert np.allclose(sim.rz(theta), c * np.eye(2) - 1j * s * sim.Z)


def test_rotation_special_angles():
    assert np.allclose(sim.rx(np.pi), -1j * sim.X)
    assert np.allclose(sim.ry(np.pi), -1j * sim.Y)
    assert np.allclose(sim.rz(np.pi), -1j * sim.Z)
    assert np.allclose(sim.rx(0.0), np.eye(2))


@pytest.mark.parametrize("theta", [0.0, 0.7, np.pi])
def test_rotations_unitary(theta):
    for U in (sim.rx(theta), sim.ry(theta), sim.rz(theta), sim.phase(theta)):
        assert np.allclose(U.conj().T @ U, np.eye(2))


def test_cnot_truth_table():
    """|c t> -> |c, t XOR c>, checked entry by entry."""
    for c, t in itertools.product((0, 1), repeat=2):
        vec = np.zeros(4)
        vec[2 * c + t] = 1.0
        out = sim.CNOT @ vec
        expect = np.zeros(4)
        expect[2 * c + (t ^ c)] = 1.0
        assert np.allclose(out, expect), f"CNOT|{c}{t}>"


def test_cz_is_diagonal_and_symmetric():
    assert np.allclose(sim.CZ, np.diag(np.diag(sim.CZ)))
    assert np.allclose(sim.CZ, sim.CZ.T)
    assert np.allclose(np.diag(sim.CZ), [1, 1, 1, -1])


# --------------------------------------------------------------------------
# section 37 -- state-vector transformation, 1 to 4 qubits
# --------------------------------------------------------------------------
def kron_all(ops):
    """Explicit tensor product in the simulator's axis-order convention.

    Axis 0 is qubit 0 and C-order flattening makes axis 0 the most significant
    bit, so qubit 0 is the *leftmost* Kronecker factor.  This helper is written
    independently of the simulator so the comparison is meaningful.
    """
    out = np.array([[1.0 + 0j]])
    for U in ops:
        out = np.kron(out, U)
    return out


@pytest.mark.parametrize("n", (1, 2, 3, 4))
@pytest.mark.parametrize("gate_name", ["X", "H", "Z", "Y"])
def test_single_gate_matches_explicit_kronecker(n, gate_name):
    """Compare tensordot application against the full operator, all positions."""
    U = sim.GATES_1Q[gate_name]
    for q in range(n):
        sv = sim.StateVector(n)
        # randomise the starting state so the check is not just on |0...0>
        rng = np.random.default_rng(100 * n + q)
        vec = rng.normal(size=2 ** n) + 1j * rng.normal(size=2 ** n)
        vec /= np.linalg.norm(vec)
        sv.psi = vec.reshape((2,) * n).copy()

        ops = [sim.I2] * n
        ops[q] = U
        expect = kron_all(ops) @ vec

        sv.apply_1q(U, q)
        assert np.allclose(sv.vector, expect), f"n={n} q={q} {gate_name}"


@pytest.mark.parametrize("n", (2, 3, 4))
def test_two_qubit_gate_matches_explicit_operator(n):
    """CNOT on every ordered qubit pair, against a permutation-built operator."""
    for q0, q1 in itertools.permutations(range(n), 2):
        rng = np.random.default_rng(1000 * n + 10 * q0 + q1)
        vec = rng.normal(size=2 ** n) + 1j * rng.normal(size=2 ** n)
        vec /= np.linalg.norm(vec)

        sv = sim.StateVector(n)
        sv.psi = vec.reshape((2,) * n).copy()
        sv.apply_2q(sim.CNOT, q0, q1)

        # build the expected operator by acting on each basis state directly
        # qubit k is axis k, so it is bit (n-1-k) of the flat index
        expect = np.zeros(2 ** n, dtype=np.complex128)
        for idx in range(2 ** n):
            bits = [(idx >> (n - 1 - k)) & 1 for k in range(n)]
            new = list(bits)
            new[q1] = bits[q1] ^ bits[q0]
            j = sum(b << (n - 1 - k) for k, b in enumerate(new))
            expect[j] += vec[idx]
        assert np.allclose(sv.vector, expect), f"n={n} CNOT({q0},{q1})"


def test_bell_state():
    """H on qubit 0 then CNOT(0 -> 1) gives (|00> + |11>)/sqrt(2)."""
    sv = sim.StateVector(2)
    sv.apply_1q(sim.H, 0).cnot(0, 1)
    expect = np.array([1, 0, 0, 1], dtype=np.complex128) / np.sqrt(2)
    assert np.allclose(sv.vector, expect)


def test_ghz_state():
    sv = sim.StateVector(3)
    sv.apply_1q(sim.H, 0).cnot(0, 1).cnot(1, 2)
    expect = np.zeros(8, dtype=np.complex128)
    expect[0] = expect[7] = 1 / np.sqrt(2)
    assert np.allclose(sv.vector, expect)


@pytest.mark.parametrize("n", (1, 2, 3, 4, 6))
def test_plus_state_matches_hadamard_layer(n):
    direct = sim.StateVector.plus_state(n)
    layered = sim.StateVector(n).apply_h_layer()
    assert np.allclose(direct.vector, layered.vector)
    assert np.allclose(direct.vector, 2 ** (-n / 2))


@pytest.mark.parametrize("n", (1, 2, 3, 5))
def test_norm_preserved_under_random_circuit(n):
    rng = np.random.default_rng(7 + n)
    sv = sim.StateVector(n)
    for _ in range(40):
        if n > 1 and rng.random() < 0.4:
            q0, q1 = rng.choice(n, size=2, replace=False)
            sv.apply_2q(sim.CNOT if rng.random() < 0.5 else sim.CZ, int(q0), int(q1))
        else:
            q = int(rng.integers(n))
            th = float(rng.uniform(0, 2 * np.pi))
            sv.apply_1q([sim.rx, sim.ry, sim.rz][int(rng.integers(3))](th), q)
    assert sv.norm() == pytest.approx(1.0, abs=1e-10)


# --------------------------------------------------------------------------
# bit-ordering convention -- the silent-bug guard
# --------------------------------------------------------------------------
def test_qubit_q_is_axis_q():
    """Setting qubit q must light up flat index 2^(n-1-q).

    Axis q is qubit q and C-order flattening puts axis 0 in the most
    significant position.  If this convention drifted, QAOA would decode
    solutions onto the wrong input variables while every energy still looked
    plausible -- so the mapping is asserted, not assumed.
    """
    n = 4
    for q in range(n):
        sv = sim.StateVector(n)
        sv.apply_1q(sim.X, q)
        idx = int(np.argmax(np.abs(sv.vector)))
        assert idx == 2 ** (n - 1 - q), f"qubit {q} -> index {idx}"


def test_index_to_bits_is_variable_ordered():
    sv = sim.StateVector(4)
    bits = sv.index_to_bits([0b0001, 0b1000, 0b0101])
    assert list(bits[0]) == [0, 0, 0, 1]
    assert list(bits[1]) == [1, 0, 0, 0]
    assert list(bits[2]) == [0, 1, 0, 1]


def test_bits_to_index_inverts_index_to_bits():
    """Round-trip guard: the two helpers must agree for every basis state."""
    for n in (1, 3, 5):
        sv = sim.StateVector(n)
        idx = np.arange(2 ** n)
        assert np.array_equal(sv.bits_to_index(sv.index_to_bits(idx)), idx)


def test_measure_recovers_deterministic_state():
    sv = sim.StateVector(3)
    sv.apply_1q(sim.X, 0).apply_1q(sim.X, 2)
    bits, counts = sv.measure(shots=500, seed=0)
    assert len(counts) == 1 and counts[0] == 500
    assert list(bits[0]) == [1, 0, 1]


def test_measure_frequencies_match_probabilities():
    sv = sim.StateVector.plus_state(3)
    bits, counts = sv.measure(shots=200_000, seed=1)
    assert counts.sum() == 200_000
    freq = counts / counts.sum()
    assert np.allclose(freq, 1 / 8, atol=5e-3)


def test_measure_is_seed_reproducible():
    sv = sim.StateVector.plus_state(4)
    a = sv.measure(shots=1000, seed=42)
    b = sv.measure(shots=1000, seed=42)
    assert np.array_equal(a[0], b[0]) and np.array_equal(a[1], b[1])


# --------------------------------------------------------------------------
# diagonal-phase shortcuts must equal their gate decompositions
# --------------------------------------------------------------------------
@pytest.mark.parametrize("theta", [0.0, 0.4, 1.7, np.pi])
@pytest.mark.parametrize("n,q0,q1", [(2, 0, 1), (3, 0, 2), (3, 2, 1), (4, 1, 3)])
def test_rzz_equals_cnot_rz_cnot(theta, n, q0, q1):
    """The fast diagonal RZZ must equal the standard decomposition exactly."""
    rng = np.random.default_rng(int(abs(theta) * 100) + n)
    vec = rng.normal(size=2 ** n) + 1j * rng.normal(size=2 ** n)
    vec /= np.linalg.norm(vec)

    fast = sim.StateVector(n)
    fast.psi = vec.reshape((2,) * n).copy()
    fast.rzz(theta, q0, q1)

    slow = sim.StateVector(n)
    slow.psi = vec.reshape((2,) * n).copy()
    slow.cnot(q0, q1)
    slow.apply_1q(sim.rz(theta), q1)
    slow.cnot(q0, q1)

    assert np.allclose(fast.vector, slow.vector, atol=1e-12)


@pytest.mark.parametrize("n", (2, 3, 4))
def test_rz_all_equals_individual_rz(n):
    rng = np.random.default_rng(n)
    thetas = rng.uniform(-np.pi, np.pi, size=n)
    a = sim.StateVector.plus_state(n)
    a.rz_all(thetas)
    b = sim.StateVector.plus_state(n)
    for q, th in enumerate(thetas):
        b.apply_1q(sim.rz(th), q)
    assert np.allclose(a.vector, b.vector)


def test_rx_all_is_the_mixer():
    n = 3
    beta = 0.37
    a = sim.StateVector.plus_state(n)
    a.rx_all(beta)
    b = sim.StateVector.plus_state(n)
    for q in range(n):
        b.apply_1q(sim.rx(2 * beta), q)
    assert np.allclose(a.vector, b.vector)


# --------------------------------------------------------------------------
# expectation values and resource accounting
# --------------------------------------------------------------------------
def test_diagonal_expectation_on_uniform_state():
    """<+|D|+> is the plain mean of the diagonal."""
    n = 4
    rng = np.random.default_rng(3)
    diag = rng.normal(size=2 ** n)
    sv = sim.StateVector.plus_state(n)
    assert sv.expectation_diagonal(diag) == pytest.approx(diag.mean())


def test_diagonal_expectation_on_basis_state():
    sv = sim.StateVector(3)
    sv.apply_1q(sim.X, 1)                     # |010> in variable order -> index 2
    diag = np.arange(8, dtype=float)
    assert sv.expectation_diagonal(diag) == pytest.approx(2.0)


def test_resource_depth_counts_parallel_layers():
    """Gates on disjoint qubits share a layer; gates sharing one do not."""
    sv = sim.StateVector(4)
    sv.apply_1q(sim.X, 0).apply_1q(sim.X, 1).apply_1q(sim.X, 2).apply_1q(sim.X, 3)
    assert sv.resources.depth == 1
    assert sv.resources.gate_count == 4
    sv.apply_1q(sim.X, 0)
    assert sv.resources.depth == 2

    chain = sim.StateVector(3)
    chain.apply_1q(sim.H, 0).cnot(0, 1).cnot(1, 2)
    assert chain.resources.depth == 3
    assert chain.resources.two_qubit_gate_count == 2


def test_rzz_charges_hardware_decomposition():
    """The simulation shortcut must not under-report hardware cost."""
    sv = sim.StateVector(2)
    sv.rzz(0.5, 0, 1)
    assert sv.resources.two_qubit_gate_count == 2, "RZZ costs two CNOTs on hardware"
    assert sv.resources.gate_count == 3


# --------------------------------------------------------------------------
# guards
# --------------------------------------------------------------------------
def test_refuses_oversized_state_with_structured_error():
    with pytest.raises(sim.SimulationResourceError) as exc:
        sim.StateVector(40)
    d = exc.value.as_dict()
    assert d["requested_qubits"] == 40
    assert d["estimated_statevector_bytes"] == 16 * 2 ** 40
    assert d["recommended_max_qubits"] == 30
    assert d["configured_memory_limit"] == sim.StateVector.DEFAULT_MEMORY_LIMIT


def test_memory_limit_is_configurable():
    """A tighter budget must lower the wall, and the error must say by how much."""
    sim.StateVector(4, memory_limit_bytes=16 * 2 ** 4)          # exactly fits
    with pytest.raises(sim.SimulationResourceError) as exc:
        sim.StateVector(5, memory_limit_bytes=16 * 2 ** 4)
    assert exc.value.recommended_max_qubits == 4


def test_rejects_nonpositive_qubit_count():
    with pytest.raises(ValueError):
        sim.StateVector(0)


def test_detects_nan_amplitude():
    sv = sim.StateVector(2)
    sv.psi[0, 0] = np.nan
    with pytest.raises(sim.NumericalError, match="non-finite"):
        sv.check_numerics()


def test_detects_inf_amplitude():
    sv = sim.StateVector(2)
    sv.psi[1, 1] = np.inf
    with pytest.raises(sim.NumericalError, match="non-finite"):
        sv.probabilities()


def test_detects_norm_drift():
    sv = sim.StateVector(2)
    sv.psi *= 1.5
    with pytest.raises(sim.NumericalError, match="norm"):
        sv.check_numerics()


def test_tolerates_float_level_drift():
    """Residual error at 1e-12 must pass; only material drift is a failure."""
    sv = sim.StateVector.plus_state(3)
    sv.psi *= (1.0 + 1e-13)
    sv.check_numerics()
    assert sv.probabilities().sum() == pytest.approx(1.0)


def test_rejects_out_of_range_qubit():
    sv = sim.StateVector(2)
    with pytest.raises(IndexError):
        sv.apply_1q(sim.X, 5)
    with pytest.raises(ValueError, match="distinct"):
        sv.apply_2q(sim.CNOT, 1, 1)


def test_max_simulable_qubits_matches_arithmetic():
    """30 qubits at 35% of 48 GiB -- the wall quoted in the report."""
    assert sim.max_simulable_qubits(48.0, 0.35) == 30
    # 2^30 complex128 = 16 GiB, which must fit inside the stated budget
    assert 16 * 2 ** 30 <= 0.35 * 48 * 2 ** 30
    assert 16 * 2 ** 31 > 0.35 * 48 * 2 ** 30
