"""Conjunctive rule arms: exactness, backward compatibility, solver agreement.

Arms turn coverage from simple incidence into a monotone DNF (disjunction of
conjunctions).  Two properties have to hold for the benchmark to remain sound:

  * an instance without arms must produce byte-identical QUBOs to before, so
    previously published results keep their meaning (spec section 22);
  * every solver must optimise the *same* arm-aware objective (spec section 2.2).

Both are enumerated here rather than argued.
"""

import itertools

import numpy as np
import pytest

from qbm import classical as cl
from qbm import qubo as qb
from qbm.instances import Instance, generate

# outcome 0: single arm {0,1}       -- conjunctive
# outcome 1: arms {0} and {2}       -- disjunctive alternatives
# outcome 2: empty arm              -- unconditional
ARMS = [(0, (0, 1)), (1, (0,)), (1, (2,)), (2, ())]
W = np.array([3.0, 2.0, 5.0])
C = np.array([1.0, 1.0, 1.0])


def _A(arms, n_in=3, n_out=3):
    A = np.zeros((n_in, n_out), dtype=np.uint8)
    for j, mem in arms:
        for i in mem:
            A[i, j] = 1
    return A


@pytest.fixture
def tiny():
    return _A(ARMS), W, C, ARMS


# ---------------------------------------------------------------- semantics
def test_instance_requires_A_to_be_the_or_flattening_of_arms():
    A = _A(ARMS)
    A[2, 0] = 1                                    # edge no arm explains
    with pytest.raises(ValueError, match="OR-flattening"):
        Instance(instance_id="t", regime="test", seed=0, A=A, w=W, c=C, arms=ARMS)


def test_arm_member_absent_from_A_is_rejected():
    with pytest.raises(ValueError, match="absent from A"):
        Instance(instance_id="t", regime="test", seed=0, A=_A(ARMS), w=W, c=C,
                 arms=ARMS + [(0, (2,))])


def test_conjunctive_arm_needs_all_members(tiny):
    A, w, c, arms = tiny
    inst = Instance(instance_id="t", regime="test", seed=0, A=A, w=w, c=c, arms=arms)
    assert not inst.covered([1, 0, 0])[0]
    assert not inst.covered([0, 1, 0])[0]
    assert inst.covered([1, 1, 0])[0]


def test_unconditional_outcome_covered_by_empty_selection(tiny):
    A, w, c, arms = tiny
    inst = Instance(instance_id="t", regime="test", seed=0, A=A, w=w, c=c, arms=arms)
    assert inst.covered(np.zeros(3, bool)).tolist() == [False, False, True]


def test_is_conjunctive_flag(tiny):
    A, w, c, arms = tiny
    inst = Instance(instance_id="t", regime="test", seed=0, A=A, w=w, c=c, arms=arms)
    assert inst.is_conjunctive
    singleton = [(1, (0,)), (1, (2,))]
    inst2 = Instance(instance_id="t", regime="test", seed=0,
                     A=_A(singleton), w=w, c=c, arms=singleton)
    assert not inst2.is_conjunctive


# ------------------------------------------------------- backward compatibility
def test_singleton_arms_reproduce_the_incidence_semantics():
    inst = generate("high_overlap", 8, 10, seed=3)
    singleton = [(int(j), (int(i),)) for i, j in np.argwhere(inst.A > 0)]
    witharms = Instance(instance_id="t", regime="test", seed=0, A=inst.A,
                        w=inst.w, c=inst.c, arms=singleton)
    rng = np.random.default_rng(0)
    for _ in range(200):
        x = rng.integers(0, 2, size=inst.n_inputs).astype(bool)
        assert (inst.covered(x) == witharms.covered(x)).all()


def test_singleton_arms_produce_an_identical_qubo():
    """Threading arms through must not perturb an instance that had none."""
    inst = generate("clustered", 7, 9, seed=5)
    singleton = [(int(j), (int(i),)) for i, j in np.argwhere(inst.A > 0)]
    q0 = qb.build_max_coverage(inst.A, inst.w, inst.c, 3, encoding="slack")
    q1 = qb.build_max_coverage(inst.A, inst.w, inst.c, 3, encoding="slack",
                               arms=singleton)
    assert q0.n_vars == q1.n_vars
    assert np.allclose(q0.Q, q1.Q)
    assert q0.offset == pytest.approx(q1.offset)


def test_multi_input_arm_costs_exactly_one_extra_variable():
    A = _A(ARMS)
    q_no = qb.build_max_coverage(A, W, C, 2, encoding="slack",
                                 arms=[(1, (0,)), (1, (2,))])
    q_yes = qb.build_max_coverage(A, W, C, 2, encoding="slack", arms=ARMS)
    assert q_yes.meta["n_arm_vars"] == 1
    assert q_yes.meta["n_unconditional_outcomes"] == 1
    # +1 arm variable, and the unconditional outcome drops its y variable
    assert q_no.meta["n_arm_vars"] == 0


# ------------------------------------------------------------------ exactness
@pytest.mark.parametrize("K", [1, 2, 3])
@pytest.mark.parametrize("beta", [0.0, 0.1])
def test_arm_qubo_minimum_equals_native_minimum(tiny, K, beta):
    """The decisive test: enumerate the whole QUBO and the native problem."""
    A, w, c, arms = tiny
    q = qb.build_max_coverage(A, w, c, K, encoding="slack", alpha=1.0,
                              beta=beta, arms=arms)
    qubo_min = min(q.energy(np.array(b, dtype=np.uint8))
                   for b in itertools.product((0, 1), repeat=q.n_vars))
    native_min = min(
        qb.native_max_coverage_objective(A, w, c, np.array(x, bool), K,
                                         alpha=1.0, beta=beta, arms=arms)[0]
        for x in itertools.product((0, 1), repeat=3) if sum(x) <= K)
    assert qubo_min == pytest.approx(native_min, abs=1e-9)


def test_unconditional_outcome_enters_as_a_constant(tiny):
    A, w, c, arms = tiny
    q = qb.build_max_coverage(A, w, c, 0, encoding="slack", arms=arms)
    # With K=0 nothing can be selected, so the only coverage is the constant.
    z = np.zeros(q.n_vars, dtype=np.uint8)
    assert q.energy(z) == pytest.approx(-w[2])


def test_arm_variable_cannot_be_set_without_its_members(tiny):
    A, w, c, arms = tiny
    q = qb.build_max_coverage(A, w, c, 2, encoding="slack", arms=arms)
    a = q.meta["arm_var_start"]
    lit = np.zeros(q.n_vars, dtype=np.uint8)
    lit[a] = 1                                     # claim the arm, select nothing
    assert q.energy(lit) > q.energy(np.zeros(q.n_vars, dtype=np.uint8))


def test_pairwise_encoding_refuses_conjunctive_arms(tiny):
    A, w, c, arms = tiny
    with pytest.raises(ValueError, match="conjunctive arm"):
        qb.build_max_coverage(A, w, c, 2, encoding="pairwise", arms=arms)


# ------------------------------------------------------------ solver agreement
@pytest.mark.parametrize("K", [1, 2, 3])
def test_every_solver_optimises_the_same_arm_aware_objective(tiny, K):
    A, w, c, arms = tiny
    exact = cl.exhaustive(A, w, c, K, arms=arms)
    ilp = cl.ilp(A, w, c, K, arms=arms)
    sa = cl.solve_via_qubo(A, w, c, K, arms=arms, seed=1, n_sweeps=1500)
    assert ilp.extra["certified_optimal"]
    assert ilp.objective_value == pytest.approx(exact.objective_value)
    assert sa.objective_value == pytest.approx(exact.objective_value)
    assert sa.covered_outcomes == exact.covered_outcomes


def test_greedy_records_that_its_bound_does_not_apply(tiny):
    A, w, c, arms = tiny
    g = cl.greedy(A, w, c, 2, arms=arms)
    assert g.extra["submodular_bound_applies"] is False
    assert g.extra["objective_is_submodular"] is False
    assert g.extra["approximation_ratio_bound"] is None


def test_greedy_stalls_where_only_a_pair_pays_and_the_optimum_does_not():
    """A pure conjunction defeats greedy: no single input has positive gain."""
    arms = [(0, (0, 1))]
    A = _A(arms, n_in=2, n_out=1)
    w = np.array([10.0])
    c = np.array([1.0, 1.0])
    g = cl.greedy(A, w, c, 2, arms=arms)
    e = cl.exhaustive(A, w, c, 2, arms=arms)
    assert g.extra["stalled_with_budget_remaining"] is True
    assert g.selected_inputs == []
    assert e.selected_inputs == [0, 1]
    assert e.objective_value < g.objective_value      # greedy strictly worse


# ------------------------------------------------------- serialization, subsets
def test_arms_survive_save_load_round_trip(tmp_path, tiny):
    A, w, c, arms = tiny
    inst = Instance(instance_id="armed", regime="test", seed=0, A=A, w=w, c=c,
                    arms=arms)
    inst.save(tmp_path)
    back = Instance.load(tmp_path / "armed")
    assert back.arms == inst.arms
    assert back.checksum() == inst.checksum()
    rng = np.random.default_rng(1)
    for _ in range(20):
        x = rng.integers(0, 2, size=3).astype(bool)
        assert (back.covered(x) == inst.covered(x)).all()


def test_checksum_changes_when_arms_change(tiny):
    A, w, c, arms = tiny
    a = Instance(instance_id="t", regime="test", seed=0, A=A, w=w, c=c, arms=arms)
    b = Instance(instance_id="t", regime="test", seed=0, A=A, w=w, c=c,
                 arms=arms + [(0, (0,))])
    assert a.checksum() != b.checksum()


def test_subinstance_drops_arms_whose_members_were_removed():
    """A conjunction is destroyed by losing a member, not weakened."""
    arms = [(0, (0, 1)), (1, (2,))]
    A = _A(arms, n_in=3, n_out=2)
    w = np.array([1.0, 1.0])
    c = np.ones(3)
    inst = Instance(instance_id="big", regime="test", seed=0, A=A, w=w, c=c,
                    arms=arms)
    sub = inst.subinstance(1)                      # keeps the single best input
    assert sub.n_inputs == 1
    # Whatever survived, every remaining arm must be fully inside the sub-instance
    for _, mem in sub.arms:
        assert all(0 <= i < sub.n_inputs for i in mem)
    # and A must still be the OR-flattening (constructor would have raised)
    assert sub.covered(np.ones(1, bool)).sum() == len(
        {j for j, _ in sub.arms})
