"""QUBO correctness and Ising-equivalence tests.

The project spec calls the tests in this file the most important in the whole
codebase (sections 34-35): if the QUBO does not reproduce the objective ordering
of the original constrained problem, every downstream comparison between QAOA
and the classical baselines is meaningless regardless of how well the optimisers
work.

Strategy throughout: enumerate *every* assignment of small instances and compare
against ``native_max_coverage_objective``, which is the sole definition of
ground truth.  Nothing here trusts the QUBO to check itself.
"""

from __future__ import annotations

import itertools
from itertools import combinations, product

import numpy as np
import pytest

from qbm import instances as ins
from qbm import qubo as qb

EXACT_ENCODINGS = ("slack", "pairwise")
ALL_REGIMES = ("low_overlap", "sparse", "degree_capped",
               "clustered", "high_overlap", "dense")


# --------------------------------------------------------------------------
# helpers
# --------------------------------------------------------------------------
def native_optimum(A, w, c, K, *, alpha=1.0, beta=0.0):
    """Brute-force optimum of the *native* constrained problem."""
    n_in = A.shape[0]
    best, best_x = np.inf, None
    for bits in product((0, 1), repeat=n_in):
        x = np.array(bits, dtype=bool)
        obj, feas = qb.native_max_coverage_objective(
            A, w, c, x, K, alpha=alpha, beta=beta)
        if feas and obj < best:
            best, best_x = obj, x
    return best, best_x


def small_instance(regime, n_in=6, n_out=7, seed=0):
    inst = ins.generate(regime, n_in, n_out, seed=seed)
    return inst.A, inst.w, inst.c


# --------------------------------------------------------------------------
# section 34 -- QUBO reproduces the native objective ordering
# --------------------------------------------------------------------------
@pytest.mark.parametrize("encoding", EXACT_ENCODINGS)
@pytest.mark.parametrize("seed", range(6))
def test_exact_encoding_recovers_native_optimum(encoding, seed):
    """The argmin of the QUBO must decode to a native optimum."""
    regime = "degree_capped" if encoding == "pairwise" else "clustered"
    A, w, c = small_instance(regime, seed=seed)
    K = 3
    q = qb.build_max_coverage(A, w, c, K, encoding=encoding)
    Z, E = q.all_energies()

    best_native, _ = native_optimum(A, w, c, K)
    x_hat = q.varmap.x_of(Z[int(np.argmin(E))])
    obj, feasible = qb.native_max_coverage_objective(A, w, c, x_hat, K)

    assert feasible, f"{encoding}: QUBO optimum violates the input budget"
    assert obj == pytest.approx(best_native), (
        f"{encoding}: decoded objective {obj} != native optimum {best_native}")


@pytest.mark.parametrize("encoding", EXACT_ENCODINGS)
def test_exact_encoding_preserves_objective_ordering(encoding):
    """Spec 34: ordering, not just the argmin.

    For every *feasible* assignment of the native problem, the QUBO energy of
    its best completion must order identically to the native objective.  A
    formulation that got the optimum right but the ordering wrong would still
    mislead every heuristic that walks the energy landscape.
    """
    regime = "degree_capped" if encoding == "pairwise" else "low_overlap"
    A, w, c = small_instance(regime, n_in=5, n_out=6, seed=1)
    K = 3
    q = qb.build_max_coverage(A, w, c, K, encoding=encoding)
    Z, E = q.all_energies()
    vm = q.varmap

    # best QUBO energy achievable for each x pattern (auxiliaries free)
    best_for_x: dict[tuple[int, ...], float] = {}
    X = Z[:, vm.x_slice]
    for key, e in zip(map(tuple, X.tolist()), E):
        if e < best_for_x.get(key, np.inf):
            best_for_x[key] = e

    native, qubo = [], []
    for bits, e in best_for_x.items():
        x = np.array(bits, dtype=bool)
        obj, feas = qb.native_max_coverage_objective(A, w, c, x, K)
        if feas:
            native.append(obj)
            qubo.append(e)

    native = np.array(native)
    qubo = np.array(qubo)
    # exact encodings must agree on the value, not merely the rank
    assert np.allclose(native, qubo), (
        f"{encoding}: energy of best completion != native objective "
        f"(max deviation {np.max(np.abs(native - qubo))})")


@pytest.mark.parametrize("regime", ALL_REGIMES)
def test_slack_encoding_exact_in_every_regime(regime):
    """The slack encoding claims exactness at any outcome degree -- verify it."""
    A, w, c = small_instance(regime, n_in=5, n_out=6, seed=3)
    K = 2
    q = qb.build_max_coverage(A, w, c, K, encoding="slack")
    if q.n_vars > 24:
        # dense regimes need one slack bit per reaching input, so the exact
        # encoding outgrows exhaustive enumeration before the *instance* looks
        # large.  That is the finding of docs/formulation.md section 6, not a
        # defect -- shrink the outcome set until it fits.
        A, w, c = small_instance(regime, n_in=5, n_out=3, seed=3)
        q = qb.build_max_coverage(A, w, c, K, encoding="slack")
        assert q.n_vars <= 24, f"still {q.n_vars} vars"
    Z, E = q.all_energies()
    best_native, _ = native_optimum(A, w, c, K)
    x_hat = q.varmap.x_of(Z[int(np.argmin(E))])
    obj, feas = qb.native_max_coverage_objective(A, w, c, x_hat, K)
    assert feas and obj == pytest.approx(best_native)


def test_pairwise_rejects_high_degree_instances():
    """The pairwise encoding must refuse instances it cannot represent.

    Silently building an inexact QUBO would be the worst possible failure mode,
    so the guard is part of the contract.
    """
    A, w, c = small_instance("dense", n_in=6, n_out=7, seed=0)
    assert A.sum(axis=0).max() > 2
    with pytest.raises(ValueError, match="degree"):
        qb.build_max_coverage(A, w, c, 3, encoding="pairwise")


# --------------------------------------------------------------------------
# the compact encoding is inexact -- and provably so
# --------------------------------------------------------------------------
def exact_quadratic_penalty_exists(d: int) -> tuple[bool, float]:
    """Does a quadratic penalty in (y, x_1..x_d) exist that is exact?

    Solves for the coefficient vector directly: the penalty must vanish on
    every feasible assignment and be non-zero on the single violating one
    (y=1, all x=0).  Returns (exists, max |P(violation)| over the null space).
    """
    nv = 1 + d
    pairs = list(combinations(range(nv), 2))
    rows_feasible, row_violation = [], None
    for bits in product((0, 1), repeat=nv):
        y, xs = bits[0], bits[1:]
        vec = ([1.0] + [float(b) for b in bits]
               + [float(bits[i] * bits[j]) for i, j in pairs])
        if y == 0 or sum(xs) >= 1:
            rows_feasible.append(vec)
        else:
            row_violation = vec

    M = np.array(rows_feasible)
    null = np.linalg.svd(M)[2][np.linalg.matrix_rank(M):]
    if null.size == 0:
        return False, 0.0
    vals = null @ np.array(row_violation)
    return bool(np.any(np.abs(vals) > 1e-9)), float(np.max(np.abs(vals)))


def test_no_exact_quadratic_penalty_above_degree_one():
    """docs/formulation.md section 3.4, verified numerically.

    At outcome degree 1 an exact quadratic penalty exists.  At degree >= 2 the
    feasible-set constraints force every coefficient to zero, so the penalty
    also vanishes on the violating assignment -- no coefficient choice can make
    the compact encoding exact.
    """
    exists, _ = exact_quadratic_penalty_exists(1)
    assert exists, "degree-1 penalty should be representable"
    for d in (2, 3, 4, 5):
        exists, magnitude = exact_quadratic_penalty_exists(d)
        assert not exists, f"unexpected exact quadratic penalty at degree {d}"
        assert magnitude == pytest.approx(0.0, abs=1e-9)


def test_compact_encoding_rewards_redundancy():
    """The concrete mechanism: a spurious reward proportional to redundancy.

    One outcome, two reaching inputs, both selected: the compact term evaluates
    to lam*(1 - 2) = -lam, i.e. it *pays* to select a redundant input.  That is
    the opposite of what a coverage objective should do.
    """
    A = np.array([[1], [1]], dtype=np.uint8)
    w = np.array([1.0])
    c = np.array([0.0, 0.0])
    lam = 10.0
    q = qb.build_max_coverage(A, w, c, K=2, encoding="penalty", lam=lam, gamma=lam)
    Z, E = q.all_energies()
    vm = q.varmap

    def energy_of(x_bits, y_bit):
        """Best energy over all completions of the given (x, y) pattern.

        Taking the *first* matching row instead of the minimum would compare
        arbitrary slack settings against each other and measure nothing.
        """
        cand = [e for z, e in zip(Z, E)
                if list(z[vm.x_slice]) == list(x_bits)
                and z[vm.y_slice][0] == y_bit]
        assert cand, "state not found"
        return min(cand)

    single = energy_of([1, 0], 1)
    both = energy_of([1, 1], 1)
    assert both < single - 1.0, (
        "compact encoding should spuriously prefer the redundant selection "
        f"(got {both} vs {single})")
    assert both == pytest.approx(single - lam)


@pytest.mark.parametrize("lam_scale", (0.5, 1.0, 10.0, 100.0))
def test_compact_encoding_stays_broken_at_any_penalty_scale(lam_scale):
    """Recovery does not improve with lam -- the signature of inexactness.

    If the compact encoding were merely under-penalised, raising lam would fix
    it.  Across a 200x range it does not, because no exact quadratic penalty
    exists at all.
    """
    failures = 0
    trials = 0
    for regime in ("degree_capped", "clustered", "high_overlap"):
        for seed in range(4):
            A, w, c = small_instance(regime, n_in=5, n_out=6, seed=seed)
            K = 2
            base = qb.penalty_bound(w, c, 1.0, 0.0)
            q = qb.build_max_coverage(A, w, c, K, encoding="penalty",
                                      lam=base * lam_scale,
                                      gamma=base * lam_scale)
            Z, E = q.all_energies()
            x_hat = q.varmap.x_of(Z[int(np.argmin(E))])
            best_native, _ = native_optimum(A, w, c, K)
            obj, feas = qb.native_max_coverage_objective(A, w, c, x_hat, K)
            trials += 1
            if not (feas and obj == pytest.approx(best_native)):
                failures += 1
    assert failures > trials // 2, (
        f"compact encoding unexpectedly recovered {trials - failures}/{trials} "
        f"optima at lam_scale={lam_scale}")


# --------------------------------------------------------------------------
# section 35 -- QUBO / Ising equivalence
# --------------------------------------------------------------------------
@pytest.mark.parametrize("n", (2, 3, 4, 5, 6, 8))
@pytest.mark.parametrize("seed", (0, 1, 2))
def test_qubo_ising_equivalence_on_every_assignment(n, seed):
    """Spec 35: QUBO energy == Ising energy + offset, for all 2^n states."""
    rng = np.random.default_rng(seed)
    M = rng.normal(scale=2.0, size=(n, n))
    q = Q = qb.Qubo.from_dense(M, offset=float(rng.normal()))
    h, J, const = q.to_ising()

    Z, E = q.all_energies()
    spins = 1.0 - 2.0 * Z                     # x = (1 - z)/2  =>  z = 1 - 2x
    ising = spins @ h + np.einsum("mi,ij,mj->m", spins, J, spins) + const
    assert np.allclose(E, ising, atol=1e-9), (
        f"max deviation {np.max(np.abs(E - ising))}")


@pytest.mark.parametrize("encoding", EXACT_ENCODINGS)
def test_ising_equivalence_on_built_problems(encoding):
    """The same check on QUBOs that came out of the builders, not random ones."""
    regime = "degree_capped" if encoding == "pairwise" else "sparse"
    A, w, c = small_instance(regime, n_in=4, n_out=5, seed=2)
    q = qb.build_max_coverage(A, w, c, 2, encoding=encoding)
    h, J, const = q.to_ising()
    Z, E = q.all_energies()
    spins = 1.0 - 2.0 * Z
    ising = spins @ h + np.einsum("mi,ij,mj->m", spins, J, spins) + const
    assert np.allclose(E, ising, atol=1e-9)


def test_ising_has_no_self_coupling():
    """J must be strictly upper-triangular: Z_i^2 = I, so J_ii is meaningless."""
    rng = np.random.default_rng(7)
    q = qb.Qubo.from_dense(rng.normal(size=(5, 5)))
    _, J, _ = q.to_ising()
    assert np.allclose(np.diag(J), 0.0)
    assert np.allclose(np.tril(J, -1), 0.0)


# --------------------------------------------------------------------------
# container invariants and regression guards (spec 41)
# --------------------------------------------------------------------------
def test_from_dense_preserves_the_function():
    """Folding to upper-triangular form must not change any energy."""
    rng = np.random.default_rng(11)
    M = rng.normal(size=(6, 6))
    q = qb.Qubo.from_dense(M)
    Z, E = q.all_energies()
    direct = np.einsum("mi,ij,mj->m", Z.astype(float), M, Z.astype(float))
    assert np.allclose(E, direct)


def test_rejects_lower_triangular_input():
    with pytest.raises(ValueError, match="upper-triangular"):
        qb.Qubo(np.array([[1.0, 0.0], [2.0, 1.0]]))


def test_budget_slack_permits_smaller_panels():
    """Regression: a bare (sum x - K)^2 penalty forbids under-using the budget.

    The whole point of the benchmark is to find *smaller* panels, so a
    formulation that penalises |S| < K would bias every algorithm identically
    and invisibly.  Here a single input already covers everything, so the
    optimum must select exactly one input even though K = 3.
    """
    A = np.ones((3, 4), dtype=np.uint8)       # every input covers every outcome
    w = np.ones(4)
    c = np.zeros(3)
    q = qb.build_max_coverage(A, w, c, K=3, encoding="slack")
    Z, E = q.all_energies()
    x_hat = q.varmap.x_of(Z[int(np.argmin(E))])
    assert x_hat.sum() >= 1
    # with zero cost, any panel covering everything ties; assert the *smallest*
    # such panel is not penalised relative to the full one
    e_one = min(e for z, e in zip(Z, E) if q.varmap.x_of(z).sum() == 1)
    e_all = min(e for z, e in zip(Z, E) if q.varmap.x_of(z).sum() == 3)
    assert e_one <= e_all + 1e-9, (
        "using fewer inputs must never cost more at equal coverage")


def test_varmap_decodes_round_trip():
    A, w, c = small_instance("clustered", n_in=5, n_out=6, seed=0)
    q = qb.build_max_coverage(A, w, c, 3, encoding="slack")
    vm = q.varmap
    assert vm.n_vars == q.n_vars
    z = np.zeros(vm.n_vars, dtype=np.uint8)
    z[[0, 2]] = 1
    x = vm.x_of(z)
    assert list(np.flatnonzero(x)) == [0, 2]


def test_energy_batch_matches_scalar():
    rng = np.random.default_rng(5)
    q = qb.Qubo.from_dense(rng.normal(size=(7, 7)), offset=1.25)
    Zb = rng.integers(0, 2, size=(20, 7))
    assert np.allclose(q.energies(Zb), [q.energy(z) for z in Zb])


def test_penalty_bound_is_strict():
    """The bound must strictly exceed the largest achievable objective gain."""
    w = np.array([1.0, 2.0, 3.0])
    c = np.array([0.5, 0.5])
    lam = qb.penalty_bound(w, c, alpha=1.0, beta=1.0)
    assert lam > w.sum() + c.sum()


def test_min_input_set_respects_coverage_floor():
    """The min-input-set mode must not return a panel below its coverage target."""
    A, w, c = small_instance("clustered", n_in=5, n_out=5, seed=4)
    inst_w = np.ones_like(w)
    target = 0.6
    q = qb.build_min_input_set(A, inst_w, c, target, encoding="slack")
    if q.n_vars > 20:
        pytest.skip(f"instance needs {q.n_vars} vars; enumeration too large")
    Z, E = q.all_energies()
    x_hat = q.varmap.x_of(Z[int(np.argmin(E))])
    covered = (A[x_hat].sum(axis=0) > 0) if x_hat.any() else np.zeros(A.shape[1], bool)
    achieved = inst_w[covered].sum() / inst_w.sum()
    assert achieved >= target - 1e-9, (
        f"coverage {achieved:.3f} below target {target}")


def test_min_input_set_rejects_inexact_encoding():
    A, w, c = small_instance("sparse", n_in=4, n_out=4, seed=0)
    with pytest.raises(NotImplementedError):
        qb.build_min_input_set(A, w, c, 0.5, encoding="penalty")


# --------------------------------------------------------------------------
# budget constraint senses -- at-most, exactly and at-least are different
# constraints and encoding one as another silently biases the optimizer
# --------------------------------------------------------------------------
def _budget_only_qubo(n_in, K, sense, reward_pairs, gamma=20.0):
    """Coverage reward on given (input, weight) pairs plus one budget block."""
    nb = qb.budget_slack_bits(K, n_in, sense)
    q = qb.Qubo.zeros(n_in + nb)
    for i, wt in reward_pairs:
        q.add_linear(i, -wt)
    qb._budget_block(q, n_in, K, gamma, n_in, nb, sense=sense)
    return q


def _best_by_cardinality(q, n_in):
    Z, E = q.all_energies()
    best = {}
    for z, e in zip(Z, E):
        k = int(z[:n_in].sum())
        best[k] = min(best.get(k, np.inf), float(e))
    return best


@pytest.mark.parametrize("sense,expected_card", [("at_most", 2), ("exactly", 3),
                                                 ("at_least", 3)])
def test_budget_senses_select_different_cardinalities(sense, expected_card):
    """Two useful inputs, budget 3: the sense alone decides the answer.

    Inputs 2 and 3 contribute nothing.  Under at-most-K the optimum uses only
    the 2 useful inputs; equality and at-least-K are obliged to pad.
    """
    q = _budget_only_qubo(4, 3, sense, [(0, 5.0), (1, 5.0)])
    Z, E = q.all_energies()
    k = int(np.argmin(E))
    assert int(Z[k][:4].sum()) == expected_card


def test_at_most_penalty_is_flat_below_budget():
    """The whole point of slack: no gradient pulling toward spending the budget."""
    best = _best_by_cardinality(_budget_only_qubo(4, 3, "at_most", []), 4)
    for k in range(0, 4):
        assert best[k] == pytest.approx(best[0]), f"|x|={k} penalised"
    assert best[4] > best[0], "over-budget must be penalised"


def test_exactly_penalty_is_a_strict_well():
    best = _best_by_cardinality(_budget_only_qubo(4, 3, "exactly", []), 4)
    assert best[3] == pytest.approx(0.0)
    for k in (0, 1, 2, 4):
        assert best[k] > best[3], f"|x|={k} not penalised under exactly-K"


def test_at_least_penalty_is_flat_above_budget():
    best = _best_by_cardinality(_budget_only_qubo(4, 3, "at_least", []), 4)
    assert best[3] == pytest.approx(best[4])
    for k in (0, 1, 2):
        assert best[k] > best[3], f"|x|={k} not penalised under at-least-K"


def test_budget_slack_bits_per_sense():
    assert qb.budget_slack_bits(3, 10, "exactly") == 0
    # at-most must represent a shortfall in 0..K
    assert 2 ** qb.budget_slack_bits(5, 10, "at_most") - 1 >= 5
    # at-least must represent an excess in 0..n_in-K
    assert 2 ** qb.budget_slack_bits(5, 10, "at_least") - 1 >= 5


def test_exactly_sense_refuses_slack_bits():
    q = qb.Qubo.zeros(4)
    with pytest.raises(ValueError, match="no slack"):
        qb._budget_block(q, 4, 2, 1.0, 4, 2, sense="exactly")


def test_unknown_budget_sense_rejected():
    q = qb.Qubo.zeros(4)
    with pytest.raises(ValueError, match="unknown budget sense"):
        qb._budget_block(q, 4, 2, 1.0, 4, 0, sense="at_most_ish")


@pytest.mark.parametrize("encoding", ["slack", "pairwise"])
def test_default_max_coverage_budget_is_at_most(encoding):
    """Regression guard on the built benchmark path, not just the helper.

    Inputs 2 and 3 cover nothing and cost nothing; if the budget were encoded as
    an equality the optimum would pad the panel out to K.
    """
    A = np.array([[1, 0], [0, 1], [0, 0], [0, 0]], np.uint8)
    q = qb.build_max_coverage(A, np.array([5.0, 5.0]), np.zeros(4), K=3,
                              encoding=encoding)
    Z, E = q.all_energies()
    x = q.varmap.x_of(Z[int(np.argmin(E))])
    assert int(x.sum()) == 2, "at-most-K budget behaving as exactly-K"
