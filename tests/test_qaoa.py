"""QAOA validation on tiny QUBOs with known optima (spec1 s38; spec3 s4, s9-11).

The strategy throughout: use problems small enough that the optimum, and often
the exact expectation value, can be written down independently of the QAOA
implementation.  Where a closed form exists it is compared against; where it
does not, the reference is exhaustive enumeration of the same QUBO.
"""

from __future__ import annotations

import numpy as np
import pytest

from qbm import qaoa as qa
from qbm import qubo as qb
from qbm import simulator as sim


def diag_qubo(linear, quadratic=None, offset=0.0):
    """Build a QUBO directly from coefficients, bypassing the benchmark layer."""
    n = len(linear)
    q = qb.Qubo.zeros(n)
    for i, v in enumerate(linear):
        q.add_linear(i, float(v))
    for (i, j), v in (quadratic or {}).items():
        q.add_quadratic(i, j, float(v))
    q.offset += offset
    return q


# --------------------------------------------------------------------------
# circuit construction
# --------------------------------------------------------------------------
def test_p1_zero_angles_leaves_uniform_state():
    """gamma=beta=0 must give |+>^n exactly: the identity check on the ansatz."""
    q = diag_qubo([1.0, -2.0], {(0, 1): 0.5})
    engine = qa.Qaoa(q)
    sv = engine.state(np.zeros(1), np.zeros(1))
    assert np.allclose(sv.vector, 2 ** (-engine.n / 2))


def test_expectation_at_zero_angles_is_mean_energy():
    """<+|H_C|+> is the unweighted mean of the QUBO diagonal."""
    q = diag_qubo([0.7, -1.3, 2.0], {(0, 2): -0.4, (1, 2): 1.1})
    engine = qa.Qaoa(q)
    assert engine.expectation(np.zeros(2)) == pytest.approx(engine.diag.mean())


def test_cost_layer_matches_gate_decomposition():
    """The diagonal-phase shortcut must equal an RZZ/RZ gate sequence.

    This is the load-bearing check on the shortcut: if it disagreed, every QAOA
    energy in the project would be subtly wrong while looking plausible.
    """
    q = diag_qubo([0.9, -0.5, 1.4], {(0, 1): 0.8, (1, 2): -1.2, (0, 2): 0.3})
    engine = qa.Qaoa(q)
    gamma = 0.37

    fast = engine.state(np.array([gamma]), np.array([0.0]))

    h, J, _ = q.to_ising()
    slow = sim.StateVector.plus_state(q.n_vars)
    for i in range(q.n_vars):
        for j in range(i + 1, q.n_vars):
            if J[i, j]:
                slow.rzz(2 * gamma * J[i, j], i, j)
    slow.rz_all(2 * gamma * h)

    # the Ising form differs from the QUBO by a constant, which is a global
    # phase here and physically irrelevant -- align it before comparing
    k = int(np.argmax(np.abs(slow.vector)))
    ratio = fast.vector[k] / slow.vector[k]
    assert abs(abs(ratio) - 1.0) < 1e-10, "shortcut changed amplitudes, not just phase"
    assert np.allclose(fast.vector, slow.vector * ratio, atol=1e-10)


def test_state_norm_preserved_across_depths():
    q = diag_qubo([1.0, -1.0, 0.5, 2.0], {(0, 1): 1.0, (2, 3): -0.5})
    engine = qa.Qaoa(q)
    rng = np.random.default_rng(0)
    for p in (1, 2, 3):
        params = rng.uniform(0, np.pi, 2 * p)
        sv = engine.state(params[:p], params[p:])
        assert sv.norm() == pytest.approx(1.0, abs=1e-10)
        sv.check_numerics()


@pytest.mark.parametrize("p", (1, 2, 3))
def test_ansatz_matches_dense_matrix_exponentials(p):
    """The strongest available reference: scipy matrix exponentials.

    ``H_C`` is built as a dense diagonal matrix and ``H_B = sum_i X_i`` as an
    explicit Kronecker sum, then both are exponentiated with ``scipy.linalg.expm``
    and applied densely.  Nothing in this reference path touches the project's
    simulator, its reshape logic or its qubit ordering, so agreement validates
    the ansatz, the angle conventions and the endianness simultaneously.
    """
    from scipy.linalg import expm

    rng = np.random.default_rng(100 + p)
    n = 4
    q = diag_qubo(rng.normal(size=n),
                  {(i, j): float(rng.normal()) for i in range(n)
                   for j in range(i + 1, n)})
    engine = qa.Qaoa(q)

    X = np.array([[0, 1], [1, 0]], complex)

    def kron_at(op, k):
        m = np.array([[1.0 + 0j]])
        for t in range(n):
            m = np.kron(m, op if t == k else np.eye(2))
        return m

    HB = sum(kron_at(X, k) for k in range(n))
    HC = np.diag(engine.diag.astype(complex))

    params = rng.uniform(0.1, 1.2, 2 * p)
    psi = np.ones(2 ** n, complex) / 2 ** (n / 2)
    for g, b in zip(params[:p], params[p:]):
        psi = expm(-1j * g * HC) @ psi
        psi = expm(-1j * b * HB) @ psi

    ours = engine.state(params[:p], params[p:]).vector
    assert np.allclose(ours, psi, atol=1e-12), \
        f"max amplitude deviation {np.abs(ours - psi).max():.2e}"
    assert engine.expectation(params) == pytest.approx(
        float(np.real(psi.conj() @ HC @ psi)), abs=1e-12)


# --------------------------------------------------------------------------
# known analytic cases
# --------------------------------------------------------------------------
def test_single_qubit_qaoa_reaches_exact_optimum():
    """One variable, H_C = -Z-like: p=1 QAOA can reach the optimum exactly.

    For a single qubit the ansatz spans enough of the Bloch sphere that the
    optimum is attainable, so anything less than the true minimum is a bug.
    """
    q = diag_qubo([-3.0])                       # E(0)=0, E(1)=-3
    engine = qa.Qaoa(q)
    rec = engine.optimize(1, optimizer="COBYLA", seed=0, n_restarts=6)
    assert rec.best_expectation == pytest.approx(-3.0, abs=1e-3)


def test_two_qubit_ferromagnet_p1_expectation_beats_random():
    """QAOA at p=1 must at least improve on the uniform-superposition energy."""
    q = diag_qubo([0.0, 0.0], {(0, 1): -2.0})   # optimum: both set, E=-2
    engine = qa.Qaoa(q)
    uniform = engine.diag.mean()
    rec = engine.optimize(1, seed=1, n_restarts=6)
    assert rec.best_expectation < uniform - 1e-6
    _, E = q.all_energies()
    assert rec.best_expectation >= E.min() - 1e-9, "below the true minimum"


@pytest.mark.parametrize("p", (1, 2, 3))
def test_expectation_never_below_true_minimum(p):
    """A variational expectation cannot beat the ground state.

    This catches sign errors, offset mishandling and Ising-conversion drift in
    one cheap assertion, at every depth.
    """
    rng = np.random.default_rng(p)
    lin = rng.normal(size=4)
    quad = {(i, j): float(rng.normal()) for i in range(4) for j in range(i + 1, 4)}
    q = diag_qubo(lin, quad, offset=float(rng.normal()))
    engine = qa.Qaoa(q)
    _, E = q.all_energies()
    rec = engine.optimize(p, seed=p, n_restarts=3)
    assert rec.best_expectation >= E.min() - 1e-9


@pytest.mark.parametrize("p", (1, 2, 3))
def test_finds_optimum_on_tiny_coverage_qubo(p):
    """End-to-end on a real benchmark QUBO: the sampled best must be optimal.

    Small enough that the certified optimum comes from enumeration of the same
    QUBO, so this tests the whole path -- builder, Ising conversion, circuit,
    sampling, decoding -- against ground truth.
    """
    A = np.array([[1, 0, 0], [0, 1, 0], [0, 0, 1], [1, 1, 0]], np.uint8)
    w = np.array([1.0, 1.0, 1.0])
    c = np.zeros(4)
    q = qb.build_max_coverage(A, w, c, K=2, encoding="pairwise")
    _, E = q.all_energies()
    opt = float(E.min())

    engine = qa.Qaoa(q)
    out = engine.run(p, shots=4000, seed=7, n_restarts=4, optimum_energy=opt)
    assert out.best_sampled_energy == pytest.approx(opt, abs=1e-9)
    assert out.optimum_probability > 0.0


# --------------------------------------------------------------------------
# reported quantities must be distinct and self-consistent (spec3 s11)
# --------------------------------------------------------------------------
def test_expectation_and_sampled_best_are_distinct_quantities():
    """The sampled best must be at least as good as the mean expectation.

    Conflating these two is the most common way to overstate QAOA: the
    expectation is an average over the whole distribution, the best sample is
    its minimum.
    """
    A = np.array([[1, 0], [0, 1], [1, 1]], np.uint8)
    q = qb.build_max_coverage(A, np.array([1.0, 1.0]), np.zeros(3), K=2,
                              encoding="pairwise")
    engine = qa.Qaoa(q)
    out = engine.run(2, shots=3000, seed=3, n_restarts=3)
    assert out.best_sampled_energy <= out.final_expectation + 1e-9


def test_probabilities_are_valid_and_ordered():
    q = diag_qubo([-1.0, -1.0], {(0, 1): 1.5})
    engine = qa.Qaoa(q)
    out = engine.run(1, shots=2000, seed=5, n_restarts=3)
    for v in (out.optimum_probability, out.near_optimum_probability,
              out.best_sample_probability):
        assert 0.0 <= v <= 1.0 + 1e-12
    # near-optimal is a superset of optimal
    assert out.near_optimum_probability >= out.optimum_probability - 1e-12


def test_feasible_probability_uses_supplied_predicate():
    """Feasibility is a property of the benchmark, so it is injected."""
    A = np.array([[1, 0], [0, 1], [1, 1]], np.uint8)
    K = 1
    q = qb.build_max_coverage(A, np.array([1.0, 1.0]), np.zeros(3), K=K,
                              encoding="pairwise")
    engine = qa.Qaoa(q)

    def feasible(z):
        return int(q.varmap.x_of(z).sum()) <= K

    out = engine.run(1, shots=2000, seed=2, feasible_fn=feasible)
    assert 0.0 <= out.feasible_probability <= 1.0
    assert not np.isnan(out.feasible_probability)

    out_none = engine.run(1, shots=500, seed=2)
    assert np.isnan(out_none.feasible_probability), "must not fabricate feasibility"


# --------------------------------------------------------------------------
# resource accounting (spec1 s43)
# --------------------------------------------------------------------------
def test_resources_charge_hardware_not_the_shortcut():
    """Reported gate counts must reflect the RZZ decomposition, not the phase trick."""
    q = diag_qubo([1.0, 1.0, 1.0], {(0, 1): 1.0, (1, 2): 1.0})
    engine = qa.Qaoa(q)
    res = qa.ising_resource_cost(engine.h, engine.J, p=1)
    assert res["n_couplings"] == 2
    assert res["two_qubit_gate_count"] == 4, "2 CNOTs per RZZ"
    assert res["parameter_count"] == 2
    # the simulator's own counter is much smaller -- that asymmetry is the point
    out = engine.run(1, shots=200, seed=0)
    assert out.resources["two_qubit_gate_count"] > out.resources["simulator_gate_count"]


def test_resources_scale_linearly_with_depth():
    q = diag_qubo([1.0, -1.0], {(0, 1): 0.5})
    engine = qa.Qaoa(q)
    r1 = qa.ising_resource_cost(engine.h, engine.J, 1, count_state_prep=False)
    r3 = qa.ising_resource_cost(engine.h, engine.J, 3, count_state_prep=False)
    assert r3["gate_count"] == 3 * r1["gate_count"]
    assert r3["two_qubit_gate_count"] == 3 * r1["two_qubit_gate_count"]
    assert r3["parameter_count"] == 3 * r1["parameter_count"]


def test_state_prep_charging_is_explicit():
    q = diag_qubo([1.0, -1.0], {(0, 1): 0.5})
    engine = qa.Qaoa(q)
    with_prep = qa.ising_resource_cost(engine.h, engine.J, 1, count_state_prep=True)
    without = qa.ising_resource_cost(engine.h, engine.J, 1, count_state_prep=False)
    assert with_prep["gate_count"] - without["gate_count"] == q.n_vars


# --------------------------------------------------------------------------
# optimizer bookkeeping (spec3 s10)
# --------------------------------------------------------------------------
@pytest.mark.parametrize("optimizer", qa.OPTIMIZERS)
def test_every_optimizer_records_required_fields(optimizer):
    q = diag_qubo([-1.0, 0.5], {(0, 1): 0.8})
    engine = qa.Qaoa(q)
    rec = engine.optimize(1, optimizer=optimizer, seed=0)
    d = rec.as_dict()
    for key in ("optimizer", "initial_parameters", "final_parameters", "iterations",
                "function_evaluations", "stopping_reason", "best_expectation",
                "seed", "runtime_ms"):
        assert key in d, f"{optimizer} missing {key}"
    assert len(rec.final_parameters) == 2
    assert rec.function_evaluations > 0
    assert rec.runtime_ms >= 0.0


def test_unknown_optimizer_rejected():
    engine = qa.Qaoa(diag_qubo([1.0]))
    with pytest.raises(ValueError, match="unknown optimizer"):
        engine.optimize(1, optimizer="magic")


def test_seed_reproduces_run():
    q = diag_qubo([-1.0, 0.5, 1.0], {(0, 1): 0.8, (1, 2): -0.3})
    engine = qa.Qaoa(q)
    a = engine.run(2, shots=1500, seed=11, n_restarts=2)
    b = engine.run(2, shots=1500, seed=11, n_restarts=2)
    assert a.final_expectation == pytest.approx(b.final_expectation)
    assert a.best_sampled_state == b.best_sampled_state
    assert a.optimizer.final_parameters == pytest.approx(b.optimizer.final_parameters)


def test_restarts_do_not_worsen_the_result():
    """More classical effort must not produce a worse expectation."""
    rng = np.random.default_rng(4)
    q = diag_qubo(rng.normal(size=4),
                  {(i, j): float(rng.normal()) for i in range(4)
                   for j in range(i + 1, 4)})
    engine = qa.Qaoa(q)
    one = engine.optimize(1, seed=9, n_restarts=1)
    many = engine.optimize(1, seed=9, n_restarts=8)
    assert many.best_expectation <= one.best_expectation + 1e-9


# --------------------------------------------------------------------------
# guards
# --------------------------------------------------------------------------
def test_oversized_qubo_refused_before_setup():
    """The memory guard must fire on construction, not deep inside a run."""
    q = qb.Qubo.zeros(40)
    with pytest.raises(sim.SimulationResourceError):
        qa.Qaoa(q)


def test_tight_memory_limit_refuses():
    q = diag_qubo([1.0] * 6)
    with pytest.raises(sim.SimulationResourceError):
        qa.Qaoa(q, memory_limit_bytes=16 * 2 ** 3)
