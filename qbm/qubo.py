"""QUBO construction, evaluation and Ising conversion for Q-BenchMed.

This module is domain-agnostic (spec section 24).  It consumes an
:class:`~qbm.instances.Instance` only through its incidence matrix, outcome
weights and input costs, and knows nothing about hematology.

Three encodings of the coverage relation are provided.  The distinction matters
because the spec (section 8) warns that inputs != qubits but does not say which
encodings are *exact*:

``slack``
    Exact for arbitrary outcome degree.  Introduces one ``y`` bit per outcome
    plus binary-encoded slack bits, so the qubit count grows fastest.

``pairwise``
    Exact, and uses ``x`` variables only, but valid only while every outcome has
    degree <= 2 (``OR(a, b) = a + b - ab`` is exactly quadratic).  This is the
    only encoding under which a flagship-shaped instance is simulable at useful
    width, which is why the ``degree_capped`` regime exists.

``penalty``
    The compact ``x + y`` form common in the QAOA literature.  It is *not*
    exact: see :func:`docs/formulation.md` and
    ``tests/test_qubo.py::test_penalty_encoding_is_inexact``.  It is implemented
    here precisely so the benchmark can quantify how wrong it is, rather than
    leaving a reader to assume it is fine.

Variable layout is explicit and stable: :class:`VarMap` records which slice of
the bit vector holds ``x``, ``y`` and slack, so a measured bitstring can always
be decoded back to a set of selected inputs.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Iterator, Literal

import numpy as np

ENCODINGS = ("slack", "pairwise", "penalty")
Encoding = Literal["slack", "pairwise", "penalty"]
Mode = Literal["max_coverage", "min_input_set"]


# --------------------------------------------------------------------------
# variable bookkeeping
# --------------------------------------------------------------------------
@dataclass(frozen=True)
class VarMap:
    """Which bit of the QUBO vector means what.

    Blocks are laid out contiguously in the order x, y, slack, budget-slack.
    Keeping this explicit (rather than implied by construction order) is what
    makes a measured bitstring decodable and is a precondition for the
    round-trip tests in ``tests/test_qubo.py``.
    """

    n_inputs: int
    n_outcomes_modelled: int = 0
    n_slack: int = 0
    n_budget_slack: int = 0
    outcome_ids: tuple[int, ...] = ()          # which outcomes got a y variable
    slack_spec: tuple[tuple[int, int, int], ...] = ()   # (outcome, start, nbits)
    budget_slack_spec: tuple[int, int] = (0, 0)         # (start, nbits)

    @property
    def n_vars(self) -> int:
        return (self.n_inputs + self.n_outcomes_modelled
                + self.n_slack + self.n_budget_slack)

    @property
    def x_slice(self) -> slice:
        return slice(0, self.n_inputs)

    @property
    def y_slice(self) -> slice:
        return slice(self.n_inputs, self.n_inputs + self.n_outcomes_modelled)

    def x_of(self, z: np.ndarray) -> np.ndarray:
        """Decode the input-selection part of a bit vector."""
        return np.asarray(z, dtype=np.uint8)[self.x_slice].astype(bool)

    def y_of(self, z: np.ndarray) -> np.ndarray:
        return np.asarray(z, dtype=np.uint8)[self.y_slice].astype(bool)

    def describe(self) -> dict:
        return {
            "n_vars": self.n_vars,
            "n_x": self.n_inputs,
            "n_y": self.n_outcomes_modelled,
            "n_slack": self.n_slack,
            "n_budget_slack": self.n_budget_slack,
        }


# --------------------------------------------------------------------------
# QUBO container
# --------------------------------------------------------------------------
@dataclass
class Qubo:
    """Upper-triangular QUBO ``min z^T Q z + offset`` over ``z in {0,1}^n``.

    Q is stored dense-upper-triangular: ``Q[i, i]`` is the linear coefficient of
    ``z_i`` (since ``z_i^2 = z_i``) and ``Q[i, j]``, ``i < j``, is the
    coefficient of ``z_i z_j``.  Dense storage is deliberate — every instance
    that fits in a statevector simulation also fits comfortably in a dense
    matrix, and it removes a class of index bugs the spec (section 41) lists as
    regression-worthy.
    """

    Q: np.ndarray
    offset: float = 0.0
    varmap: VarMap | None = None
    meta: dict = field(default_factory=dict)

    def __post_init__(self) -> None:
        self.Q = np.asarray(self.Q, dtype=np.float64)
        if self.Q.ndim != 2 or self.Q.shape[0] != self.Q.shape[1]:
            raise ValueError(f"Q must be square, got {self.Q.shape}")
        if np.any(np.tril(self.Q, -1) != 0):
            raise ValueError("Q must be upper-triangular; use Qubo.from_dense()")

    # -- construction ------------------------------------------------------
    @classmethod
    def zeros(cls, n: int, **kw) -> "Qubo":
        return cls(np.zeros((n, n)), **kw)

    @classmethod
    def from_dense(cls, M: np.ndarray, **kw) -> "Qubo":
        """Fold an arbitrary square matrix into upper-triangular form.

        ``z^T M z`` is unchanged by symmetrising the off-diagonal, so the folded
        matrix represents exactly the same function.
        """
        M = np.asarray(M, dtype=np.float64)
        U = np.triu(M) + np.tril(M, -1).T
        return cls(U, **kw)

    @property
    def n_vars(self) -> int:
        return self.Q.shape[0]

    def add_linear(self, i: int, c: float) -> None:
        self.Q[i, i] += c

    def add_quadratic(self, i: int, j: int, c: float) -> None:
        if i == j:
            self.Q[i, i] += c
        else:
            a, b = (i, j) if i < j else (j, i)
            self.Q[a, b] += c

    # -- evaluation --------------------------------------------------------
    def energy(self, z: np.ndarray) -> float:
        z = np.asarray(z, dtype=np.float64)
        return float(z @ self.Q @ z) + self.offset

    def energies(self, Z: np.ndarray) -> np.ndarray:
        """Energies of a batch of bit vectors, ``Z`` of shape (m, n)."""
        Z = np.asarray(Z, dtype=np.float64)
        return np.einsum("mi,ij,mj->m", Z, self.Q, Z) + self.offset

    def all_energies(self) -> tuple[np.ndarray, np.ndarray]:
        """Every energy in lexicographic bitstring order (small n only).

        Returns ``(Z, E)``.  This is the workhorse of the correctness suite: the
        spec (section 34) requires that the QUBO reproduce the objective
        ordering of the original formulation on every assignment.
        """
        n = self.n_vars
        if n > 24:
            raise ValueError(f"refusing to enumerate 2^{n} states")
        Z = ((np.arange(1 << n)[:, None] >> np.arange(n)[::-1]) & 1).astype(np.uint8)
        return Z, self.energies(Z)

    def diagonal_energies(self) -> np.ndarray:
        """Energy of every state, for use as a diagonal Hamiltonian.

        The QUBO is diagonal in the computational basis, which is what lets the
        QAOA cost layer be applied as a phase multiplication rather than a gate
        sequence.
        """
        return self.all_energies()[1]

    # -- Ising conversion --------------------------------------------------
    def to_ising(self) -> tuple[np.ndarray, np.ndarray, float]:
        """Convert to ``H = sum_i h_i Z_i + sum_{i<j} J_ij Z_i Z_j + const``.

        Uses ``x_i = (1 - z_i) / 2`` where ``z_i`` is the spin (spec section
        12).  Under this convention spin ``+1`` means the bit is 0.  The
        equivalence ``QUBO(x) == Ising(z) + const`` is asserted for every
        assignment in ``tests/test_qubo.py`` (spec section 35).
        """
        Q = self.Q
        n = self.n_vars
        a = np.diag(Q).copy()                     # linear bit coefficients
        B = np.triu(Q, 1)                         # quadratic bit coefficients

        # x_i x_j = (1 - z_i)(1 - z_j)/4 = (1 - z_i - z_j + z_i z_j)/4
        J = B / 4.0
        h = -a / 2.0 - (B.sum(axis=1) + B.sum(axis=0)) / 4.0
        const = a.sum() / 2.0 + B.sum() / 4.0 + self.offset
        return h, J, float(const)

    def ising_energy(self, spins: np.ndarray) -> float:
        h, J, const = self.to_ising()
        s = np.asarray(spins, dtype=np.float64)
        return float(h @ s + s @ J @ s + const)


# --------------------------------------------------------------------------
# penalty coefficient derivation
# --------------------------------------------------------------------------
def coverage_reward_scale(w: np.ndarray) -> float:
    """The largest reward any single outcome can contribute."""
    return float(np.max(w)) if w.size else 0.0


def penalty_bound(w: np.ndarray, c: np.ndarray, alpha: float, beta: float) -> float:
    """A penalty coefficient guaranteeing no feasible-to-infeasible crossover.

    A constraint penalty is *safe* when violating any single constraint costs
    more than the largest possible objective gain from violating it.  For the
    coverage constraints the maximum gain from falsely claiming outcomes is
    ``alpha * sum(w)`` and the maximum gain from ignoring input cost is
    ``beta * sum(c)``; a penalty strictly exceeding their sum can never be
    profitable to pay.  This is conservative by design — a tight bound would
    have to be re-derived per instance, and a too-small penalty produces
    silently infeasible optima, which is the failure mode the spec's section 41
    regression list is written to catch.
    """
    return float(alpha * np.sum(w) + beta * np.sum(c)) + 1.0


def _binary_slack_bits(cap: int) -> int:
    """Bits needed to represent any integer in ``[0, cap]``."""
    return 0 if cap <= 0 else int(cap).bit_length()


# --------------------------------------------------------------------------
# builders
# --------------------------------------------------------------------------
def build_max_coverage(
    A: np.ndarray,
    w: np.ndarray,
    c: np.ndarray,
    K: int,
    *,
    encoding: Encoding = "slack",
    alpha: float = 1.0,
    beta: float = 0.0,
    gamma: float | None = None,
    lam: float | None = None,
) -> Qubo:
    """Maximum weighted outcome coverage under an input budget (spec 9.1).

    Native problem::

        max   alpha * sum_j w_j y_j  -  beta * sum_i c_i x_i
        s.t.  y_j <= sum_i A_ij x_i          (an outcome counts only if reached)
              sum_i x_i <= K                 (input budget)

    Minimised as a QUBO, so the coverage reward enters with a negative sign.
    """
    A = np.asarray(A, dtype=np.float64)
    w = np.asarray(w, dtype=np.float64)
    c = np.asarray(c, dtype=np.float64)
    n_in, n_out = A.shape
    if encoding not in ENCODINGS:
        raise ValueError(f"unknown encoding {encoding!r}; choose from {ENCODINGS}")

    lam = penalty_bound(w, c, alpha, beta) if lam is None else float(lam)
    gamma = lam if gamma is None else float(gamma)

    if encoding == "pairwise":
        return _build_pairwise(A, w, c, K, alpha=alpha, beta=beta, gamma=gamma)
    return _build_with_y(A, w, c, K, encoding=encoding,
                         alpha=alpha, beta=beta, gamma=gamma, lam=lam)


BudgetSense = Literal["at_most", "exactly", "at_least"]

#: Slack sign per budget sense.  ``at_most`` needs ``sum x + s = K`` (slack
#: absorbs the shortfall); ``at_least`` needs ``sum x - s = K``; ``exactly``
#: admits no slack at all.  These are three genuinely different constraints and
#: are never interchangeable -- encoding an at-most budget as an equality biases
#: the optimizer toward spending the whole budget on inputs that add nothing.
_BUDGET_SLACK_SIGN: dict[str, float] = {"at_most": +1.0, "exactly": 0.0,
                                       "at_least": -1.0}


def budget_slack_bits(K: int, n_in: int, sense: BudgetSense) -> int:
    """Slack bits needed to represent a budget of the given sense.

    ``exactly`` needs none.  ``at_most`` must represent a shortfall in
    ``0..K``; ``at_least`` an excess in ``0..n_in - K``.
    """
    if sense == "exactly":
        return 0
    span = K if sense == "at_most" else max(n_in - K, 0)
    return _binary_slack_bits(span)


def _budget_block(q: Qubo, n_in: int, K: int, gamma: float,
                  slack_start: int, n_bits: int,
                  sense: BudgetSense = "at_most") -> None:
    """Add ``gamma * (sum_i x_i + sigma*s - K)^2`` with binary-encoded slack.

    The default budget is an *inequality*, so a bare ``(sum x - K)^2`` term
    would penalise using fewer than K inputs -- wrong for the min-input-set mode
    and subtly wrong even for max-coverage, since it forbids a cheaper equally
    good panel.  The slack variable absorbs the difference, leaving the penalty
    exactly flat across all feasible cardinalities.

    Passing ``sense="exactly"`` reproduces the equality form deliberately, for
    benchmarks whose constraint really is exactly-K.  The distinction is
    explicit at the call site so it cannot happen by accident.
    """
    if sense not in _BUDGET_SLACK_SIGN:
        raise ValueError(f"unknown budget sense {sense!r}")
    sigma = _BUDGET_SLACK_SIGN[sense]
    if sense == "exactly" and n_bits:
        raise ValueError("exactly-K budget takes no slack bits")
    coeffs = np.concatenate([np.ones(n_in), sigma * 2.0 ** np.arange(n_bits)])
    idx = np.concatenate([np.arange(n_in), slack_start + np.arange(n_bits)])
    # (sum_k a_k v_k - K)^2 = sum_k a_k^2 v_k + 2 sum_{k<l} a_k a_l v_k v_l
    #                         - 2K sum_k a_k v_k + K^2
    for k, (ak, ik) in enumerate(zip(coeffs, idx)):
        q.add_linear(int(ik), gamma * (ak * ak - 2.0 * K * ak))
        for al, il in zip(coeffs[k + 1:], idx[k + 1:]):
            q.add_quadratic(int(ik), int(il), gamma * 2.0 * ak * al)
    q.offset += gamma * K * K


def _build_with_y(A, w, c, K, *, encoding, alpha, beta, gamma, lam) -> Qubo:
    n_in, n_out = A.shape
    deg = A.sum(axis=0).astype(int)                 # inputs reaching each outcome

    n_budget_bits = _binary_slack_bits(K)
    slack_spec: list[tuple[int, int, int]] = []
    cursor = n_in + n_out
    if encoding == "slack":
        # y_j <= sum_i A_ij x_i  <=>  sum_i A_ij x_i - y_j - s_j = 0,
        # s_j in [0, deg_j - 1] when y_j may be 1.
        for j in range(n_out):
            nb = _binary_slack_bits(max(deg[j] - 1, 0))
            slack_spec.append((j, cursor, nb))
            cursor += nb
    n_slack = cursor - (n_in + n_out)
    budget_start = cursor
    n_vars = cursor + n_budget_bits

    vm = VarMap(n_inputs=n_in, n_outcomes_modelled=n_out, n_slack=n_slack,
                n_budget_slack=n_budget_bits,
                outcome_ids=tuple(range(n_out)),
                slack_spec=tuple(slack_spec),
                budget_slack_spec=(budget_start, n_budget_bits))
    q = Qubo.zeros(n_vars, varmap=vm,
                   meta={"mode": "max_coverage", "encoding": encoding,
                         "K": K, "alpha": alpha, "beta": beta,
                         "gamma": gamma, "lam": lam})

    # objective: reward coverage, charge input cost
    for j in range(n_out):
        q.add_linear(n_in + j, -alpha * w[j])
    for i in range(n_in):
        q.add_linear(i, beta * c[i])

    if encoding == "slack":
        # lam * (sum_i A_ij x_i - y_j - s_j)^2 -- exact equality constraint
        for j, start, nb in slack_spec:
            terms = [(1.0, i) for i in np.flatnonzero(A[:, j])]
            terms.append((-1.0, n_in + j))
            terms += [(-(2.0 ** b), start + b) for b in range(nb)]
            for k, (ak, ik) in enumerate(terms):
                q.add_linear(int(ik), lam * ak * ak)
                for al, il in terms[k + 1:]:
                    q.add_quadratic(int(ik), int(il), lam * 2.0 * ak * al)
    else:  # "penalty" -- compact, inexact
        # lam * sum_j y_j * (1 - sum_i A_ij x_i) as a quadratic form.  This
        # penalises claiming an unreached outcome, but for deg_j >= 2 it also
        # *rewards* claiming an outcome reached by several selected inputs,
        # which is what breaks exactness.  See docs/formulation.md.
        for j in range(n_out):
            q.add_linear(n_in + j, lam)
            for i in np.flatnonzero(A[:, j]):
                q.add_quadratic(int(i), n_in + j, -lam)

    _budget_block(q, n_in, K, gamma, budget_start, n_budget_bits)
    return q


def _build_pairwise(A, w, c, K, *, alpha, beta, gamma) -> Qubo:
    """Exact x-only encoding, valid while every outcome degree <= 2.

    For an outcome reached by inputs a, b the coverage indicator is exactly
    ``x_a + x_b - x_a x_b``; for degree 1 it is ``x_a``; degree 0 outcomes are
    unreachable and contribute a constant.  No ``y`` variables and no slack are
    needed, so the qubit count is ``n_inputs`` plus budget slack -- the only
    encoding that keeps a flagship-shaped instance inside a statevector.
    """
    n_in, n_out = A.shape
    deg = A.sum(axis=0).astype(int)
    if deg.max(initial=0) > 2:
        raise ValueError(
            f"pairwise encoding requires max outcome degree <= 2, got {deg.max()}; "
            "use encoding='slack' or a degree_capped instance")

    n_budget_bits = _binary_slack_bits(K)
    vm = VarMap(n_inputs=n_in, n_outcomes_modelled=0, n_slack=0,
                n_budget_slack=n_budget_bits,
                budget_slack_spec=(n_in, n_budget_bits))
    q = Qubo.zeros(n_in + n_budget_bits, varmap=vm,
                   meta={"mode": "max_coverage", "encoding": "pairwise",
                         "K": K, "alpha": alpha, "beta": beta, "gamma": gamma})

    for j in range(n_out):
        idx = np.flatnonzero(A[:, j])
        if idx.size == 1:
            q.add_linear(int(idx[0]), -alpha * w[j])
        elif idx.size == 2:
            a, b = int(idx[0]), int(idx[1])
            q.add_linear(a, -alpha * w[j])
            q.add_linear(b, -alpha * w[j])
            q.add_quadratic(a, b, alpha * w[j])
    for i in range(n_in):
        q.add_linear(i, beta * c[i])

    _budget_block(q, n_in, K, gamma, n_in, n_budget_bits)
    return q


def build_min_input_set(
    A: np.ndarray,
    w: np.ndarray,
    c: np.ndarray,
    coverage_target: float,
    *,
    encoding: Encoding = "slack",
    beta: float = 1.0,
    lam: float | None = None,
    mu: float | None = None,
) -> Qubo:
    """Minimum-cost input set achieving a weighted coverage target (spec 9.2).

    Native problem::

        min   beta * sum_i c_i x_i
        s.t.  sum_j w_j y_j >= coverage_target * sum_j w_j
              y_j <= sum_i A_ij x_i

    The coverage requirement is an inequality over a weighted sum, so it needs
    its own slack block; ``mu`` is its penalty coefficient.
    """
    A = np.asarray(A, dtype=np.float64)
    w = np.asarray(w, dtype=np.float64)
    c = np.asarray(c, dtype=np.float64)
    n_in, n_out = A.shape
    target = float(coverage_target) * float(w.sum())

    lam = penalty_bound(w, c, 1.0, beta) if lam is None else float(lam)
    mu = lam if mu is None else float(mu)

    if encoding != "slack":
        raise NotImplementedError(
            "min_input_set is implemented for the exact slack encoding only; "
            "the compact form cannot represent the weighted coverage floor "
            "without distorting it (docs/formulation.md)")

    deg = A.sum(axis=0).astype(int)
    slack_spec: list[tuple[int, int, int]] = []
    cursor = n_in + n_out
    for j in range(n_out):
        nb = _binary_slack_bits(max(deg[j] - 1, 0))
        slack_spec.append((j, cursor, nb))
        cursor += nb
    n_slack = cursor - (n_in + n_out)

    # coverage floor: sum_j w_j y_j - t - s = 0, s in [0, sum w - t]
    cov_start = cursor
    cov_bits = _binary_slack_bits(int(np.ceil(w.sum() - target)))
    n_vars = cursor + cov_bits

    vm = VarMap(n_inputs=n_in, n_outcomes_modelled=n_out, n_slack=n_slack + cov_bits,
                outcome_ids=tuple(range(n_out)), slack_spec=tuple(slack_spec))
    q = Qubo.zeros(n_vars, varmap=vm,
                   meta={"mode": "min_input_set", "encoding": encoding,
                         "coverage_target": coverage_target, "beta": beta,
                         "lam": lam, "mu": mu})

    for i in range(n_in):
        q.add_linear(i, beta * c[i])

    for j, start, nb in slack_spec:
        terms = [(1.0, i) for i in np.flatnonzero(A[:, j])]
        terms.append((-1.0, n_in + j))
        terms += [(-(2.0 ** b), start + b) for b in range(nb)]
        for k, (ak, ik) in enumerate(terms):
            q.add_linear(int(ik), lam * ak * ak)
            for al, il in terms[k + 1:]:
                q.add_quadratic(int(ik), int(il), lam * 2.0 * ak * al)

    terms = [(float(w[j]), n_in + j) for j in range(n_out)]
    terms += [(-(2.0 ** b), cov_start + b) for b in range(cov_bits)]
    for k, (ak, ik) in enumerate(terms):
        q.add_linear(int(ik), mu * (ak * ak - 2.0 * target * ak))
        for al, il in terms[k + 1:]:
            q.add_quadratic(int(ik), int(il), mu * 2.0 * ak * al)
    q.offset += mu * target * target
    return q


# --------------------------------------------------------------------------
# native (non-QUBO) objective -- the ground-truth reference
# --------------------------------------------------------------------------
def native_max_coverage_objective(A, w, c, x, K, *, alpha=1.0, beta=0.0
                                  ) -> tuple[float, bool]:
    """Objective and feasibility of ``x`` under the *original* formulation.

    Everything the QUBO does is checked against this function, never the other
    way round.  Returned objective is the minimisation-sense value so it is
    directly comparable to a QUBO energy.
    """
    A = np.asarray(A); w = np.asarray(w, float); c = np.asarray(c, float)
    x = np.asarray(x, dtype=bool)
    covered = (A[x].sum(axis=0) > 0) if x.any() else np.zeros(A.shape[1], bool)
    obj = -alpha * float(w[covered].sum()) + beta * float(c[x].sum())
    return obj, bool(x.sum() <= K)


def encoding_var_count(n_inputs: int, n_outcomes: int, deg: np.ndarray,
                       K: int, encoding: Encoding) -> int:
    """Variable count without building the matrix (for the sizing figure)."""
    nb_budget = _binary_slack_bits(K)
    if encoding == "pairwise":
        return n_inputs + nb_budget
    if encoding == "penalty":
        return n_inputs + n_outcomes + nb_budget
    slack = sum(_binary_slack_bits(max(int(d) - 1, 0)) for d in deg)
    return n_inputs + n_outcomes + slack + nb_budget
