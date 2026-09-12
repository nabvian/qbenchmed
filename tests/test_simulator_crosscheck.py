"""Cross-check the state-vector simulator against Qiskit (spec section 37, 6).

Qiskit is a *development validation reference only*: the project's simulator
must not depend on it at runtime, so these tests skip cleanly when it is absent.

The comparison has to bridge a convention difference, and that bridge is the
scientifically interesting part of this file.  Qiskit is little-endian (its
qubit 0 is the least significant bit of the statevector index); this project
uses tensor-axis order, where qubit 0 is the *most* significant bit.  Comparing
raw amplitude arrays would therefore fail even for a perfectly correct
simulator, and "fixing" that by flipping our convention would reintroduce the
decode bug the main suite guards.  Instead each test maps amplitudes through the
explicit bit permutation, so both the physics *and* the stated convention are
verified at once.
"""

from __future__ import annotations

import numpy as np
import pytest

from qbm import simulator as sim

qiskit = pytest.importorskip("qiskit", reason="Qiskit is an optional dev reference")
from qiskit import QuantumCircuit                      # noqa: E402
from qiskit.quantum_info import Statevector            # noqa: E402


def qiskit_to_axis_order(vec: np.ndarray, n: int) -> np.ndarray:
    """Reindex a Qiskit statevector into this project's axis ordering.

    Qiskit index bit k is qubit k (little-endian); ours is bit (n-1-k).  The map
    is therefore a bit-reversal of the flat index, applied explicitly rather
    than by any simulator-supplied helper.
    """
    out = np.empty_like(vec)
    for idx in range(2 ** n):
        bits = [(idx >> k) & 1 for k in range(n)]        # bits[k] = qubit k
        ours = sum(b << (n - 1 - k) for k, b in enumerate(bits))
        out[ours] = vec[idx]
    return out


def assert_same_state(ours: np.ndarray, qc: QuantumCircuit, n: int, *, atol=1e-10):
    ref = qiskit_to_axis_order(np.asarray(Statevector(qc).data), n)
    # global phase is physically irrelevant; align it before comparing
    k = int(np.argmax(np.abs(ref)))
    if abs(ref[k]) > 1e-12:
        ref = ref * (ours[k] / ref[k]) / abs(ours[k] / ref[k])
    assert np.allclose(ours, ref, atol=atol), f"max dev {np.abs(ours-ref).max():.2e}"


# --------------------------------------------------------------------------
def test_bell_state_matches_qiskit():
    sv = sim.StateVector(2)
    sv.apply_1q(sim.H, 0).cnot(0, 1)
    qc = QuantumCircuit(2)
    qc.h(0)
    qc.cx(0, 1)
    assert_same_state(sv.vector, qc, 2)


def test_ghz_matches_qiskit():
    sv = sim.StateVector(3)
    sv.apply_1q(sim.H, 0).cnot(0, 1).cnot(1, 2)
    qc = QuantumCircuit(3)
    qc.h(0)
    qc.cx(0, 1)
    qc.cx(1, 2)
    assert_same_state(sv.vector, qc, 3)


@pytest.mark.parametrize("theta", [0.0, 0.3, 1.1, np.pi / 2, np.pi, 2.7])
@pytest.mark.parametrize("axis", ["rx", "ry", "rz"])
def test_single_rotations_match_qiskit(theta, axis):
    n = 3
    for q in range(n):
        sv = sim.StateVector(n)
        sv.apply_1q(sim.H, 0)                    # non-trivial starting state
        sv.apply_1q(getattr(sim, axis)(theta), q)
        qc = QuantumCircuit(n)
        qc.h(0)
        getattr(qc, axis)(theta, q)
        assert_same_state(sv.vector, qc, n)


@pytest.mark.parametrize("theta", [0.0, 0.45, 1.9, np.pi])
@pytest.mark.parametrize("q0,q1", [(0, 1), (1, 0), (0, 2), (2, 1)])
def test_rzz_matches_qiskit(theta, q0, q1):
    """Our diagonal RZZ against Qiskit's own RZZGate, on all qubit pairs."""
    n = 3
    sv = sim.StateVector.plus_state(n)
    sv.rzz(theta, q0, q1)
    qc = QuantumCircuit(n)
    for q in range(n):
        qc.h(q)
    qc.rzz(theta, q0, q1)
    assert_same_state(sv.vector, qc, n)


def test_cz_matches_qiskit():
    n = 3
    sv = sim.StateVector.plus_state(n)
    sv.cz(0, 2)
    qc = QuantumCircuit(n)
    for q in range(n):
        qc.h(q)
    qc.cz(0, 2)
    assert_same_state(sv.vector, qc, n)


def test_mixer_layer_matches_qiskit():
    """The QAOA mixer exp(-i beta sum X) as Qiskit RX(2 beta) on each qubit."""
    n = 4
    beta = 0.41
    sv = sim.StateVector.plus_state(n)
    sv.rx_all(beta)
    qc = QuantumCircuit(n)
    for q in range(n):
        qc.h(q)
    for q in range(n):
        qc.rx(2 * beta, q)
    assert_same_state(sv.vector, qc, n)


def test_full_qaoa_layer_matches_qiskit():
    """A complete p=1 cost+mixer layer for a small Ising model.

    This is the real target of the cross-check: if the cost layer, the mixer,
    the angle conventions and the qubit ordering all agree with an independent
    implementation on a full layer, the QAOA built on top is trustworthy.
    """
    n = 4
    gamma, beta = 0.63, 0.29
    J = {(0, 1): 0.8, (1, 2): -0.5, (0, 3): 1.2, (2, 3): 0.4}
    h = {0: 0.3, 1: -0.7, 2: 0.0, 3: 0.9}

    sv = sim.StateVector.plus_state(n)
    for (a, b), jab in J.items():
        sv.rzz(2 * gamma * jab, a, b)
    sv.rz_all(np.array([2 * gamma * h.get(q, 0.0) for q in range(n)]))
    sv.rx_all(beta)

    qc = QuantumCircuit(n)
    for q in range(n):
        qc.h(q)
    for (a, b), jab in J.items():
        qc.rzz(2 * gamma * jab, a, b)
    for q in range(n):
        if h.get(q, 0.0):
            qc.rz(2 * gamma * h[q], q)
    for q in range(n):
        qc.rx(2 * beta, q)

    assert_same_state(sv.vector, qc, n)


def test_random_circuit_matches_qiskit():
    """Random gate sequences, so the agreement is not specific to nice circuits."""
    n = 4
    rng = np.random.default_rng(11)
    sv = sim.StateVector(n)
    qc = QuantumCircuit(n)
    for _ in range(60):
        kind = rng.integers(5)
        if kind == 0:
            q = int(rng.integers(n)); sv.apply_1q(sim.H, q); qc.h(q)
        elif kind == 1:
            q = int(rng.integers(n)); sv.apply_1q(sim.X, q); qc.x(q)
        elif kind == 2:
            q = int(rng.integers(n)); th = float(rng.uniform(0, 2 * np.pi))
            sv.apply_1q(sim.ry(th), q); qc.ry(th, q)
        elif kind == 3:
            a, b = (int(v) for v in rng.choice(n, size=2, replace=False))
            sv.cnot(a, b); qc.cx(a, b)
        else:
            a, b = (int(v) for v in rng.choice(n, size=2, replace=False))
            th = float(rng.uniform(0, 2 * np.pi))
            sv.rzz(th, a, b); qc.rzz(th, a, b)
    assert_same_state(sv.vector, qc, n, atol=1e-9)
