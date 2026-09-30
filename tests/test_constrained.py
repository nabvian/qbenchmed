"""The constrained-mixer ansatz: subspace, objective, and the phase bug it once had.

The claims this module makes are all checkable on instances small enough to
enumerate: the XY gate conserves Hamming weight, the Dicke state is the uniform
superposition it is supposed to be, the exactly-K subspace does not lose the
at-most-K optimum, and the coverage diagonal is the objective the certified
solver optimises.  Each is tested against something outside the ansatz.
"""

from __future__ import annotations

from math import comb

import numpy as np
import pytest

from qbm import classical as cl
from qbm import constrained as cn
from qbm import instances as ins


def _weights(n: int) -> np.ndarray:
    idx = np.arange(2 ** n)
    return np.stack(np.unravel_index(idx, (2,) * n), axis=-1).sum(axis=1)


@pytest.mark.parametrize("theta", [0.0, 0.3, 1.1, np.pi / 2, 2.9])
def test_xy_gate_is_unitary_and_weight_preserving(theta):
    g = cn._xy_gate(theta)
    assert np.allclose(g @ g.conj().T, np.eye(4))
    # |00> and |11> are fixed; the weight-1 block only mixes |01>,|10>
    assert np.isclose(abs(g[0, 0]), 1) and np.isclose(abs(g[3, 3]), 1)
    assert np.allclose(g[[0, 3]][:, [1, 2]], 0) and np.allclose(g[[1, 2]][:, [0, 3]], 0)


@pytest.mark.parametrize("n,k", [(3, 1), (5, 2), (6, 3), (7, 0), (7, 7)])
def test_dicke_state_is_uniform_over_weight_k(n, k):
    p = cn.dicke_state(n, k).probabilities()
    w = _weights(n)
    assert np.isclose(p.sum(), 1.0)
    assert np.allclose(p[w != k], 0)
    assert np.allclose(p[w == k], 1 / comb(n, k))


def test_ring_mixer_never_leaves_the_subspace():
    rng = np.random.default_rng(0)
    sv = cn.dicke_state(8, 3)
    for beta in rng.uniform(0, np.pi, 5):
        cn.apply_ring_mixer(sv, float(beta))
    p = sv.probabilities()
    assert p[_weights(8) != 3].sum() < 1e-12


@pytest.mark.parametrize("seed", range(5))
def test_coverage_diagonal_matches_the_certified_solver(seed):
    inst = ins.generate("degree_capped", 10, 13, seed=seed)
    K = 3
    at_k, le_k = cn.optimum_at_weight(cn.coverage_diagonal(inst.A, inst.w), K)
    assert np.isclose(at_k, le_k), "the exactly-K subspace lost the at-most-K optimum"
    certified = cl.ilp(inst.A, inst.w, inst.c, K, arms=inst.arms)
    assert np.isclose(-at_k, certified.coverage_fraction * inst.w.sum())


def test_cost_layer_uses_the_unwrapped_phase():
    """Regression: exp(-i E) ** gamma wraps E into (-pi, pi] and is wrong.

    Coverage energies here reach -12, well outside that interval.  The state
    after one cost layer (no mixer) must carry exp(-i gamma E) exactly.
    """
    inst = ins.generate("degree_capped", 8, 11, seed=1)
    K = 3
    qa = cn.ConstrainedQaoa(inst.A, inst.w, K)
    assert qa.diag.min() < -np.pi, "instance too small to exercise the bug"
    gamma = 0.37
    sv = qa.state(np.array([gamma]), np.array([0.0]))
    expected = (cn.dicke_state(8, K).psi * np.exp(-1j * gamma * qa.diag)).reshape(-1)
    assert np.allclose(sv.vector, expected, atol=1e-12)


def test_every_sample_is_feasible_and_scored_against_ground_truth():
    inst = ins.generate("degree_capped", 8, 11, seed=2)
    K = 3
    qa = cn.ConstrainedQaoa(inst.A, inst.w, K)
    opt, _ = cn.optimum_at_weight(qa.diag, K)
    out = qa.run(1, shots=2000, seed=0, optimum_energy=opt, grid_resolution=16)
    assert out.feasible_probability == pytest.approx(1.0, abs=1e-12)
    assert 0.0 < out.optimum_probability <= 1.0
    assert out.best_sampled_energy >= opt - 1e-9


def test_resource_count_charges_the_cost_layer_and_the_prep():
    inst = ins.generate("degree_capped", 10, 13, seed=0)
    r = cn.constrained_resource_cost(cn.coverage_diagonal(inst.A, inst.w), 3, p=2)
    assert r["cost_layer_two_qubit_per_layer"] == cn.CNOT_PER_RZZ * r["n_coverage_couplings"]
    assert r["mixer_two_qubit_per_layer"] == cn.CNOT_PER_XY * 10
    assert r["two_qubit_gate_count"] == 2 * (r["cost_layer_two_qubit_per_layer"]
                                             + r["mixer_two_qubit_per_layer"])
    assert (r["two_qubit_gate_count_with_prep"]
            == r["two_qubit_gate_count"] + r["dicke_prep"]["prep_two_qubit_gates"])


def test_ising_terms_agree_with_the_pair_count_when_coverage_is_pairwise():
    """On a degree-<=2, arm-free instance the exact expansion is pairs and singles."""
    inst = ins.generate("degree_capped", 10, 13, seed=3)
    terms = cn.ising_terms(cn.coverage_diagonal(inst.A, inst.w))
    pairs = {tuple(np.flatnonzero(inst.A[:, j])) for j in range(inst.A.shape[1])
             if inst.A[:, j].sum() == 2}
    assert max(terms) <= 2
    assert terms.get(2, 0) == len(pairs)


def test_arms_are_covered_only_when_every_member_is_selected():
    """coverage_diagonal with arms must agree with qubo.coverage_mask everywhere."""
    from qbm import qubo as qb
    A = np.array([[1, 1], [1, 0], [0, 1]], dtype=float)       # 3 inputs, 2 outcomes
    w = np.array([2.0, 5.0])
    arms = [(0, (0, 1)), (1, (2,)), (1, (0, 1))]              # outcome 0 = 0 AND 1
    diag = cn.coverage_diagonal(A, w, arms).reshape(-1)
    for idx in range(8):
        x = np.array(np.unravel_index(idx, (2, 2, 2)), dtype=bool)
        expected = -(qb.coverage_mask(A, x, arms) * w).sum()
        assert diag[idx] == expected, f"x={x.astype(int)}"


def test_conjunctive_arms_produce_higher_order_terms():
    A = np.ones((3, 1))
    diag = cn.coverage_diagonal(A, np.ones(1), arms=[(0, (0, 1, 2))])
    assert cn.ising_terms(diag).get(3, 0) == 1          # a three-way AND is a 3-body term
