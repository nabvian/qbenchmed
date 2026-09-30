"""Exports: the LP file round-trips, and every emitted circuit is the circuit.

Each circuit is a single gate list applied two ways -- to this simulator, and
emitted as OpenQASM 2.0.  The first group checks the gate list against the
simulator's own ansatz states.  The second, which needs Qiskit, loads the
emitted QASM into an independent toolchain and requires the same statevector.
That second check is what caught a phase bug in the constrained simulator, so
it stays.
"""

from __future__ import annotations

import numpy as np
import pytest

from qbm import constrained as cn
from qbm import instances as ins
from qbm import interop as io
from qbm import qubo as qb
from qbm.qaoa import Qaoa


def _align(ref: np.ndarray, got: np.ndarray) -> np.ndarray:
    k = int(np.argmax(np.abs(ref)))
    return got * (ref[k] / got[k]) / abs(ref[k] / got[k])


def _instance(seed: int, n: int = 7, K: int = 3):
    inst = ins.generate("degree_capped", n, round(n * 88 / 66), seed=seed)
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, K, encoding="pairwise",
                              arms=inst.arms)
    return inst, q, K


@pytest.mark.parametrize("seed", range(3))
def test_lp_round_trip_reproduces_every_energy(seed):
    _, q, _ = _instance(seed)
    Q = io.lp_to_qubo_matrix(io.qubo_to_lp(q), q.n_vars)
    Z, E = q.all_energies()
    Zf = Z.astype(float)
    assert np.allclose(np.einsum("si,ij,sj->s", Zf, Q, Zf) + q.offset, E)


@pytest.mark.parametrize("n", range(2, 8))
def test_dicke_circuit_prepares_the_dicke_state(n):
    for k in range(n + 1):
        ref = cn.dicke_state(n, k).vector
        got = io.apply_gates(n, io.dicke_gates(n, k)).vector
        assert np.allclose(_align(ref, got), ref, atol=1e-9), f"n={n} k={k}"


@pytest.mark.parametrize("p", [1, 2, 3])
def test_circuits_match_the_simulator_ansatz(p):
    rng = np.random.default_rng(p)
    for seed in range(3):
        inst, q, K = _instance(seed)
        g, b = rng.uniform(0, 2 * np.pi, p), rng.uniform(0, np.pi, p)
        ref = Qaoa(q).state(g, b).vector
        got = io.apply_gates(q.n_vars, io.penalty_qaoa_gates(q, g, b)).vector
        assert np.allclose(_align(ref, got), ref, atol=1e-8)
        ref = cn.ConstrainedQaoa(inst.A, inst.w, K).state(g, b).vector
        got = io.apply_gates(7, io.constrained_qaoa_gates(inst.A, inst.w, K, g, b)).vector
        assert np.allclose(_align(ref, got), ref, atol=1e-8)


def test_higher_degree_coverage_is_refused_not_truncated():
    A = np.zeros((4, 1))
    A[:3, 0] = 1
    with pytest.raises(ValueError, match="degree 3"):
        io.coverage_qubo(A, np.ones(1))


def test_emitted_qasm_uses_only_the_portable_gate_set():
    inst, q, K = _instance(0)
    text = io.to_qasm2(7, io.constrained_qaoa_gates(inst.A, inst.w, K, [0.4], [0.9]))
    ops = {line.split("(")[0].split(" ")[0] for line in text.splitlines()[4:]
           if line and not line.startswith("//")}
    assert ops <= {"h", "x", "rx", "ry", "rz", "cx", "measure"}


# -- independent toolchain ------------------------------------------------
qiskit = pytest.importorskip("qiskit", reason="Qiskit is an optional dev reference")
from qiskit import qasm2, transpile                     # noqa: E402
from qiskit.quantum_info import Statevector             # noqa: E402


def _qiskit_state(qasm: str, n: int) -> np.ndarray:
    qc = qasm2.loads(qasm)
    qc.remove_final_measurements()
    vec = np.asarray(Statevector(qc).data)
    out = np.empty_like(vec)
    for idx in range(2 ** n):                            # little-endian -> ours
        out[sum(((idx >> k) & 1) << (n - 1 - k) for k in range(n))] = vec[idx]
    return out


@pytest.mark.parametrize("seed", range(3))
def test_qiskit_reproduces_both_ansatzes(seed):
    rng = np.random.default_rng(seed)
    inst, q, K = _instance(seed)
    g, b = rng.uniform(0, 2 * np.pi, 2), rng.uniform(0, np.pi, 2)
    ours = Qaoa(q).state(g, b).vector
    theirs = _qiskit_state(io.to_qasm2(q.n_vars, io.penalty_qaoa_gates(q, g, b)), q.n_vars)
    assert np.allclose(_align(ours, theirs), ours, atol=1e-8)
    ours = cn.ConstrainedQaoa(inst.A, inst.w, K).state(g, b).vector
    theirs = _qiskit_state(io.to_qasm2(7, io.constrained_qaoa_gates(inst.A, inst.w, K, g, b)), 7)
    assert np.allclose(_align(ours, theirs), ours, atol=1e-8)


@pytest.mark.parametrize("label,gates,expected", [
    ("xy", io._xy(0.7, 0, 1), cn.CNOT_PER_XY),
    ("rzz", io._rzz(0.7, 0, 1), cn.CNOT_PER_RZZ),
])
def test_resource_constants_match_qiskit_synthesis(label, gates, expected):
    qc = qasm2.loads(io.to_qasm2(2, gates, measure=False))
    t = transpile(qc, basis_gates=["cx", "rz", "sx", "x"], optimization_level=3)
    assert t.count_ops().get("cx", 0) == expected, label
