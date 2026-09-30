"""Benchmark instance generation for Q-BenchMed.

An instance is a bipartite incidence structure between *inputs* (the binary
decision variables) and *outcomes* (the targets to be covered), plus outcome
weights and input costs.

    A[i, j] = 1  <=>  input i contributes to outcome j

The generator is domain-agnostic by design (spec section 24): nothing here knows
that hematology exists.  A domain adapter supplies A, w, c and metadata; the
`heme_shaped` regime below is a *synthetic stand-in* whose degree structure
imitates a biomarker panel, used until the real 66x88 export is available.

Structure regimes (spec section 19) exist because combinatorial difficulty
depends on structure, not only on variable count.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np

SCHEMA_VERSION = "qbm-instance/1.0"

REGIMES = (
    "low_overlap",
    "high_overlap",
    "clustered",
    "sparse",
    "dense",
    "degree_capped",
    "heme_shaped",
)

WEIGHT_SCHEMES = ("uniform", "rare_weighted", "priority_weighted")


# --------------------------------------------------------------------------
# container
# --------------------------------------------------------------------------
@dataclass
class Instance:
    """A frozen benchmark instance.

    Attributes
    ----------
    A : (n_inputs, n_outcomes) uint8 incidence matrix
    w : (n_outcomes,) float outcome weights, all > 0
    c : (n_inputs,) float input costs, all > 0
    """

    instance_id: str
    regime: str
    seed: int
    A: np.ndarray
    w: np.ndarray
    c: np.ndarray
    weight_scheme: str = "uniform"
    input_names: list[str] = field(default_factory=list)
    outcome_names: list[str] = field(default_factory=list)
    notes: str = ""
    # Conjunctive rule arms (spec section 21/22).  `None` means pure binary
    # incidence: every edge of A is its own arm, which is the historical
    # behaviour.  Otherwise each entry is (outcome_index, input_indices) and
    #
    #     outcome j is covered  <=>  some arm of j has ALL its inputs selected
    #
    # so coverage is a disjunction of conjunctions (monotone DNF) rather than
    # simple incidence.  An arm with an EMPTY input tuple is always satisfied
    # and marks an outcome a rule engine emits unconditionally.
    arms: list[tuple[int, tuple[int, ...]]] | None = None

    # ---- shape helpers ---------------------------------------------------
    @property
    def n_inputs(self) -> int:
        return int(self.A.shape[0])

    @property
    def n_outcomes(self) -> int:
        return int(self.A.shape[1])

    def __post_init__(self) -> None:
        self.A = np.asarray(self.A, dtype=np.uint8)
        self.w = np.asarray(self.w, dtype=float)
        self.c = np.asarray(self.c, dtype=float)
        if self.A.ndim != 2:
            raise ValueError(f"A must be 2-D, got shape {self.A.shape}")
        if self.w.shape != (self.n_outcomes,):
            raise ValueError(f"w has shape {self.w.shape}, expected {(self.n_outcomes,)}")
        if self.c.shape != (self.n_inputs,):
            raise ValueError(f"c has shape {self.c.shape}, expected {(self.n_inputs,)}")
        if not np.all(self.w > 0):
            raise ValueError("outcome weights must be strictly positive")
        if not np.all(self.c > 0):
            raise ValueError("input costs must be strictly positive")
        if not self.input_names:
            self.input_names = [f"input_{i:03d}" for i in range(self.n_inputs)]
        if not self.outcome_names:
            self.outcome_names = [f"outcome_{j:03d}" for j in range(self.n_outcomes)]
        self._arm_cache = None
        if self.arms is not None:
            self.arms = self._normalise_arms(self.arms)

    def _normalise_arms(self, arms) -> list[tuple[int, tuple[int, ...]]]:
        """Canonicalise arms and enforce agreement with the incidence matrix.

        A and arms must describe the same edge set: A is the OR-flattening of
        the arms.  Enforcing that here means `covered()` and every statistic
        derived from A refer to one problem, so a solver that reads A cannot
        silently optimise a different instance from one that reads arms.
        """
        out = []
        for entry in arms:
            j, members = entry
            j = int(j)
            if not 0 <= j < self.n_outcomes:
                raise ValueError(f"arm outcome index {j} out of range")
            mem = tuple(sorted({int(i) for i in members}))
            for i in mem:
                if not 0 <= i < self.n_inputs:
                    raise ValueError(f"arm input index {i} out of range")
                if not self.A[i, j]:
                    raise ValueError(
                        f"arm ({self.outcome_names[j]}) references input "
                        f"{self.input_names[i]} that is absent from A")
            out.append((j, mem))
        out = sorted(set(out))
        # Every incidence edge must be explained by at least one arm.
        explained = np.zeros_like(self.A, dtype=bool)
        for j, mem in out:
            for i in mem:
                explained[i, j] = True
        missing = np.argwhere((self.A > 0) & ~explained)
        if missing.size:
            i, j = missing[0]
            raise ValueError(
                f"A has edge ({self.input_names[i]} -> {self.outcome_names[j]}) "
                f"that appears in no arm; A must be the OR-flattening of arms")
        return out

    @property
    def n_arms(self) -> int:
        return len(self.arm_list())

    def arm_list(self) -> list[tuple[int, tuple[int, ...]]]:
        """Arms, synthesising singleton arms from A when none were supplied."""
        if self.arms is not None:
            return self.arms
        return [(int(j), (int(i),)) for i, j in np.argwhere(self.A > 0)]

    @property
    def is_conjunctive(self) -> bool:
        """True when some arm needs more than one input simultaneously."""
        return self.arms is not None and any(len(m) != 1 for _, m in self.arms)

    def _arm_matrix(self):
        """(membership matrix, arm->outcome) with the empty-arm mask folded in."""
        if self._arm_cache is None:
            arms = self.arm_list()
            M = np.zeros((len(arms), self.n_inputs), dtype=bool)
            owner = np.zeros(len(arms), dtype=np.int64)
            for a, (j, mem) in enumerate(arms):
                owner[a] = j
                M[a, list(mem)] = True
            self._arm_cache = (M, owner)
        return self._arm_cache

    # ---- structural statistics ------------------------------------------
    @property
    def outcome_degree(self) -> np.ndarray:
        """Number of inputs that can cover each outcome."""
        return self.A.sum(axis=0).astype(int)

    @property
    def input_degree(self) -> np.ndarray:
        """Number of outcomes each input contributes to."""
        return self.A.sum(axis=1).astype(int)

    @property
    def density(self) -> float:
        return float(self.A.mean())

    @property
    def reachable_outcomes(self) -> np.ndarray:
        """Outcomes with at least one contributing input (others are never coverable)."""
        return np.flatnonzero(self.outcome_degree > 0)

    def jaccard_overlap(self) -> np.ndarray:
        """Pairwise Jaccard similarity of input coverage sets.

        High values mean inputs are redundant with each other, which is the
        structural property that makes set-cover style problems hard.
        """
        B = self.A.astype(bool)
        inter = (B.astype(np.int32) @ B.T.astype(np.int32)).astype(float)
        deg = B.sum(axis=1).astype(float)
        union = deg[:, None] + deg[None, :] - inter
        with np.errstate(divide="ignore", invalid="ignore"):
            J = np.where(union > 0, inter / union, 0.0)
        return J

    def mean_offdiag_overlap(self) -> float:
        J = self.jaccard_overlap()
        n = J.shape[0]
        if n < 2:
            return 0.0
        return float((J.sum() - np.trace(J)) / (n * (n - 1)))

    # ---- objective evaluation (the ground-truth definition) --------------
    def covered(self, x: np.ndarray) -> np.ndarray:
        """Boolean mask of outcomes covered by input selection x.

        This is the single definition of coverage in the framework; every
        solver and the QUBO builder score against it (spec section 2.2).
        """
        x = np.asarray(x, dtype=bool)
        if self.arms is None:
            if not x.any():
                return np.zeros(self.n_outcomes, bool)
            return self.A[x].sum(axis=0) > 0
        M, owner = self._arm_matrix()
        # An arm is satisfied iff it has no unselected member.
        satisfied = ~(M & ~x[None, :]).any(axis=1)
        y = np.zeros(self.n_outcomes, bool)
        y[owner[satisfied]] = True
        return y

    def coverage_weight(self, x: np.ndarray) -> float:
        """Total weight of outcomes covered -- the quantity maximized in mode A."""
        return float(self.w[self.covered(x)].sum())

    def coverage_fraction(self, x: np.ndarray) -> float:
        return self.coverage_weight(x) / float(self.w.sum())

    def input_cost(self, x: np.ndarray) -> float:
        return float(self.c[np.asarray(x, dtype=bool)].sum())

    # ---- provenance ------------------------------------------------------
    def checksum(self) -> str:
        h = hashlib.sha256()
        h.update(np.ascontiguousarray(self.A).tobytes())
        h.update(np.ascontiguousarray(np.round(self.w, 10)).tobytes())
        h.update(np.ascontiguousarray(np.round(self.c, 10)).tobytes())
        if self.arms is not None:
            h.update(repr(self.arms).encode())
        return h.hexdigest()[:16]

    def metadata(self) -> dict:
        od = self.outcome_degree
        return {
            "schema_version": SCHEMA_VERSION,
            "instance_id": self.instance_id,
            "regime": self.regime,
            "seed": self.seed,
            "n_inputs": self.n_inputs,
            "n_outcomes": self.n_outcomes,
            "weight_scheme": self.weight_scheme,
            "density": round(self.density, 6),
            "n_edges": int(self.A.sum()),
            "outcome_degree_min": int(od.min()),
            "outcome_degree_max": int(od.max()),
            "outcome_degree_mean": round(float(od.mean()), 4),
            "n_unreachable_outcomes": int((od == 0).sum()),
            "input_degree_mean": round(float(self.input_degree.mean()), 4),
            "mean_pairwise_overlap": round(self.mean_offdiag_overlap(), 6),
            "n_arms": self.n_arms,
            "max_arm_size": int(max((len(m) for _, m in self.arm_list()), default=0)),
            "is_conjunctive": self.is_conjunctive,
            "n_unconditional_outcomes": int(
                len({j for j, m in self.arm_list() if not m})),
            "checksum": self.checksum(),
            "notes": self.notes,
        }

    # ---- serialization ---------------------------------------------------
    def save(self, directory: str | Path) -> Path:
        d = Path(directory) / self.instance_id
        d.mkdir(parents=True, exist_ok=True)
        np.savetxt(d / "incidence.csv", self.A, fmt="%d", delimiter=",")
        # %.17g is the shortest format that round-trips float64 exactly; the
        # checksum in metadata.json is verified on load, so any lossy format
        # here turns a saved instance into an unloadable one.
        np.savetxt(d / "outcome_weights.csv", self.w, fmt="%.17g", delimiter=",")
        np.savetxt(d / "input_costs.csv", self.c, fmt="%.17g", delimiter=",")
        (d / "inputs.txt").write_text("\n".join(self.input_names) + "\n")
        (d / "outcomes.txt").write_text("\n".join(self.outcome_names) + "\n")
        if self.arms is not None:
            lines = ["outcome_index,input_indices"]
            lines += [f"{j}," + " ".join(str(i) for i in mem) for j, mem in self.arms]
            (d / "arms.csv").write_text("\n".join(lines) + "\n")
        (d / "metadata.json").write_text(json.dumps(self.metadata(), indent=2) + "\n")
        return d

    @classmethod
    def load(cls, directory: str | Path) -> "Instance":
        d = Path(directory)
        meta = json.loads((d / "metadata.json").read_text())
        A = np.loadtxt(d / "incidence.csv", delimiter=",", dtype=np.uint8, ndmin=2)
        w = np.loadtxt(d / "outcome_weights.csv", delimiter=",", ndmin=1)
        c = np.loadtxt(d / "input_costs.csv", delimiter=",", ndmin=1)
        arms = None
        arms_path = d / "arms.csv"
        if arms_path.exists():
            arms = []
            for line in arms_path.read_text().splitlines()[1:]:
                if not line.strip():
                    continue
                head, _, tail = line.partition(",")
                arms.append((int(head), tuple(int(t) for t in tail.split())))
        inst = cls(
            instance_id=meta["instance_id"],
            regime=meta["regime"],
            seed=meta["seed"],
            A=A,
            w=w,
            c=c,
            weight_scheme=meta.get("weight_scheme", "uniform"),
            input_names=(d / "inputs.txt").read_text().split(),
            outcome_names=(d / "outcomes.txt").read_text().split(),
            notes=meta.get("notes", ""),
            arms=arms,
        )
        if inst.checksum() != meta["checksum"]:
            raise ValueError(
                f"checksum mismatch for {meta['instance_id']}: "
                f"{inst.checksum()} != {meta['checksum']} (instance file was modified)"
            )
        return inst

    def subinstance(self, n_inputs: int, n_outcomes: int | None = None,
                    seed: int | None = None, instance_id: str | None = None) -> "Instance":
        """Take a structured sub-instance, for the scaling ladder (spec section 18).

        Inputs are chosen by descending degree with a seeded shuffle inside ties,
        so the sub-instance keeps hub structure rather than becoming a random
        sparse remnant.  Outcomes are restricted to those still reachable.
        """
        rng = np.random.default_rng(self.seed if seed is None else seed)
        order = np.lexsort((rng.random(self.n_inputs), -self.input_degree))
        keep_i = np.sort(order[:n_inputs])
        A = self.A[keep_i]
        reach = np.flatnonzero(A.sum(axis=0) > 0)
        if n_outcomes is not None and len(reach) > n_outcomes:
            deg = A[:, reach].sum(axis=0)
            o_order = np.lexsort((rng.random(len(reach)), -deg))
            reach = np.sort(reach[o_order[:n_outcomes]])
        sub_arms = None
        if self.arms is not None:
            # An arm is a conjunction: dropping any member destroys the arm
            # rather than weakening it, so a sub-instance keeps only arms whose
            # every member survives.  Reachability and A are then rebuilt from
            # the survivors, otherwise A would assert coverage routes that no
            # remaining arm can deliver.
            imap = {int(i): k for k, i in enumerate(keep_i)}
            kept = [(j, tuple(imap[i] for i in mem)) for j, mem in self.arms
                    if all(i in imap for i in mem)]
            n_arms_by_outcome = np.zeros(self.n_outcomes, dtype=int)
            for j, _ in kept:
                n_arms_by_outcome[j] += 1
            reach = np.flatnonzero(n_arms_by_outcome > 0)
            if n_outcomes is not None and len(reach) > n_outcomes:
                o_order = np.lexsort((rng.random(len(reach)),
                                      -n_arms_by_outcome[reach]))
                reach = np.sort(reach[o_order[:n_outcomes]])
            jmap = {int(j): k for k, j in enumerate(reach)}
            sub_arms = [(jmap[j], mem) for j, mem in kept if j in jmap]
            A = np.zeros((len(keep_i), len(reach)), dtype=np.uint8)
            for j, mem in sub_arms:
                for i in mem:
                    A[i, j] = 1
        else:
            A = A[:, reach]
        return Instance(
            instance_id=instance_id or f"{self.instance_id}-n{n_inputs}",
            regime=self.regime,
            seed=self.seed if seed is None else seed,
            A=A,
            w=self.w[reach],
            c=self.c[keep_i],
            weight_scheme=self.weight_scheme,
            input_names=[self.input_names[i] for i in keep_i],
            outcome_names=[self.outcome_names[j] for j in reach],
            notes=f"sub-instance of {self.instance_id}",
            arms=sub_arms,
        )


# --------------------------------------------------------------------------
# weights and costs
# --------------------------------------------------------------------------
def make_weights(A: np.ndarray, scheme: str, rng: np.random.Generator) -> np.ndarray:
    """Outcome importance weights.

    All schemes are defined *before* any algorithm sees the instance (spec 9.4).

    uniform            : every outcome equally important
    rare_weighted      : weight inversely proportional to outcome degree, so
                         outcomes reachable from few inputs matter more
    priority_weighted  : a seeded 20% of outcomes designated high priority (x4)
    """
    n_out = A.shape[1]
    if scheme == "uniform":
        return np.ones(n_out)
    if scheme == "rare_weighted":
        deg = A.sum(axis=0).astype(float)
        return 1.0 + 3.0 * (1.0 / np.maximum(deg, 1.0))
    if scheme == "priority_weighted":
        w = np.ones(n_out)
        k = max(1, int(round(0.2 * n_out)))
        w[rng.choice(n_out, size=k, replace=False)] = 4.0
        return w
    raise ValueError(f"unknown weight scheme {scheme!r}; expected one of {WEIGHT_SCHEMES}")


def make_costs(n_inputs: int, rng: np.random.Generator, spread: float = 3.0) -> np.ndarray:
    """Per-input acquisition cost, log-uniform on [1, spread].

    Costs are *declared here*, before results are seen (spec 9.3 warns against
    inventing costs after the fact).  They are used only by the cost-constrained
    benchmark mode; the coverage modes ignore them.
    """
    return np.exp(rng.uniform(0.0, np.log(spread), size=n_inputs))


# --------------------------------------------------------------------------
# structure regimes
# --------------------------------------------------------------------------
def _repair(A: np.ndarray, rng: np.random.Generator) -> np.ndarray:
    """Guarantee every outcome has >=1 input and every input has >=1 outcome.

    Unreachable outcomes are not wrong, but they add a constant to every
    objective value and silently deflate reported coverage fractions, so the
    generator removes that nuisance term rather than leaving it in.
    """
    n_in, n_out = A.shape
    dead_out = np.flatnonzero(A.sum(axis=0) == 0)
    if dead_out.size:
        A[rng.integers(0, n_in, size=dead_out.size), dead_out] = 1
    dead_in = np.flatnonzero(A.sum(axis=1) == 0)
    if dead_in.size:
        A[dead_in, rng.integers(0, n_out, size=dead_in.size)] = 1
    return A


def generate(
    regime: str,
    n_inputs: int,
    n_outcomes: int,
    seed: int,
    weight_scheme: str = "uniform",
    instance_id: str | None = None,
) -> Instance:
    """Generate a synthetic instance in the named structure regime."""
    if regime not in REGIMES:
        raise ValueError(f"unknown regime {regime!r}; expected one of {REGIMES}")
    rng = np.random.default_rng(seed)
    A = np.zeros((n_inputs, n_outcomes), dtype=np.uint8)

    if regime == "low_overlap":
        # each outcome reachable from ~1-2 inputs, inputs partition the outcome
        # space -> near-disjoint coverage sets, greedy should do well
        owner = rng.permutation(np.arange(n_outcomes) % n_inputs)
        A[owner, np.arange(n_outcomes)] = 1
        extra = rng.random(n_outcomes) < 0.15
        A[rng.integers(0, n_inputs, size=int(extra.sum())), np.flatnonzero(extra)] = 1

    elif regime == "high_overlap":
        # many inputs cover the same outcomes: redundancy is the difficulty
        A = (rng.random((n_inputs, n_outcomes)) < 0.35).astype(np.uint8)

    elif regime == "clustered":
        # inputs form functional groups; each group has an affinity block of
        # outcomes plus weak cross-talk.  Produces correlated redundancy, the
        # structure most likely to separate heuristics from the optimum.
        n_groups = max(2, int(round(np.sqrt(n_inputs))))
        gi = rng.integers(0, n_groups, size=n_inputs)
        go = rng.integers(0, n_groups, size=n_outcomes)
        same = gi[:, None] == go[None, :]
        p = np.where(same, 0.55, 0.03)
        A = (rng.random((n_inputs, n_outcomes)) < p).astype(np.uint8)

    elif regime == "sparse":
        A = (rng.random((n_inputs, n_outcomes)) < 0.06).astype(np.uint8)

    elif regime == "dense":
        A = (rng.random((n_inputs, n_outcomes)) < 0.60).astype(np.uint8)

    elif regime == "degree_capped":
        # every outcome reachable from exactly 2 inputs.  This regime exists for
        # a purely mathematical reason: at outcome degree <= 2 the coverage
        # constraint admits an *exact* QUBO on the x variables alone
        # (OR(a,b) = a + b - ab), so QAOA needs n_inputs qubits instead of
        # n_inputs + n_outcomes + slack.  It is the only regime in which the
        # flagship-style problem is simulable at useful size.
        #
        # The cap must hold *by construction*, not by post-hoc repair: the
        # pairwise encoding is only exact while max outcome degree <= 2, so one
        # repaired edge would silently invalidate it.  Inputs are dealt
        # round-robin from a reshuffled pool, which also guarantees every input
        # appears at least once whenever 2 * n_outcomes >= n_inputs, so _repair
        # has nothing to add.
        deg = min(2, n_inputs)
        pool, ptr = rng.permutation(n_inputs), 0
        for j in range(n_outcomes):
            chosen: list[int] = []
            while len(chosen) < deg:
                if ptr >= n_inputs:
                    pool, ptr = rng.permutation(n_inputs), 0
                cand = int(pool[ptr])
                ptr += 1
                if cand not in chosen:
                    chosen.append(cand)
            A[chosen, j] = 1

    elif regime == "heme_shaped":
        A = _heme_shaped(n_inputs, n_outcomes, rng)

    A = _repair(A, rng)
    w = make_weights(A, weight_scheme, rng)
    c = make_costs(n_inputs, rng)
    iid = instance_id or (
        f"QBM-SYN-{regime.upper().replace('_', '')}-n{n_inputs}m{n_outcomes}-s{seed}"
    )
    return Instance(
        instance_id=iid, regime=regime, seed=seed, A=A, w=w, c=c,
        weight_scheme=weight_scheme,
        notes=f"synthetic instance, regime={regime}",
    )


def _heme_shaped(n_inputs: int, n_outcomes: int, rng: np.random.Generator) -> np.ndarray:
    """Incidence structure imitating a biomarker-panel knowledge base.

    Three properties are imposed, chosen to match how such knowledge bases
    actually look rather than to make any algorithm win:

    1. A few *hub* inputs (think CBC indices) participate in many outcomes.
    2. Most inputs are specialised, touching a handful of outcomes.
    3. Outcome in-degree is right-skewed: most outcomes are defined by a small
       number of inputs, a few are broadly determined.

    This is a STAND-IN for the real 66x88 export.  It carries no clinical
    meaning and supports no clinical claim; it exists so the pipeline can be
    exercised and calibrated on a plausibly-shaped instance.
    """
    n_hub = max(1, int(round(0.08 * n_inputs)))
    hubs = rng.choice(n_inputs, size=n_hub, replace=False)

    # target in-degree per outcome: lognormal, clipped to [1, n_inputs // 2]
    target = np.clip(
        np.round(rng.lognormal(mean=np.log(4.0), sigma=0.75, size=n_outcomes)),
        1, max(1, n_inputs // 2),
    ).astype(int)

    # inputs are sampled per outcome with hub-boosted propensity
    prop = np.ones(n_inputs)
    prop[hubs] = 6.0
    prop = prop / prop.sum()

    A = np.zeros((n_inputs, n_outcomes), dtype=np.uint8)
    for j in range(n_outcomes):
        k = min(int(target[j]), n_inputs)
        chosen = rng.choice(n_inputs, size=k, replace=False, p=prop)
        A[chosen, j] = 1
    return A


class HemeDomainUnavailable(ImportError):
    """Raised when the QBMED-HEME-001 domain module cannot be imported.

    It ships with this package, so a normal checkout never sees this. A partial
    install or a vendored subset can. Everything else works without it,
    including every quantum experiment: the QAOA comparison and the noise sweep
    both run on generated instances and never load a domain.
    """


def conjunctive_panel(n_inputs: int = 66, n_outcomes: int = 88, seed: int = 11,
                      *, weight_scheme: str = "uniform",
                      instance_id: str = "QBM-SYN-CONJ") -> Instance:
    """A synthetic panel instance with conjunctive rule arms.

    `generate` produces binary incidence: every edge stands alone, so coverage
    is submodular and greedy carries its usual guarantee.  Real rule sets are
    not like that, and the property that makes them interesting -- outcomes
    that fire only on a combination of inputs -- cannot be expressed by an
    incidence matrix at all.

    This builds the missing case.  The shape is calibrated to the published
    QBMED-HEME-001 benchmark so that a reader without access to that instance
    can still exercise the same behaviour: hub inputs with high degree, most
    outcomes carrying one arm and a minority carrying several, arm sizes
    skewed towards two and three, a few outcomes reachable by nothing, and a
    couple fired unconditionally.

    It is a structural imitation and nothing more.  It contains no clinical
    content, no biomarker identities and no knowledge of any kind, and a result
    computed on it says nothing about the real instance beyond the fact that
    both are conjunctive.
    """
    rng = np.random.default_rng(seed)

    # A few hub inputs carry most of the degree, as index parameters do in a
    # real panel; the rest are specialised.
    n_hub = max(2, n_inputs // 11)
    hubs = rng.choice(n_inputs, size=n_hub, replace=False)
    weights = np.full(n_inputs, 1.0)
    weights[hubs] = 12.0
    weights /= weights.sum()

    # Arm sizes follow the measured distribution of the reference instance.
    sizes, freqs = np.array([1, 2, 3, 4, 5, 6]), np.array([30, 44, 31, 7, 3, 1])
    size_p = freqs / freqs.sum()
    # Most outcomes are defined by one arm; a minority by several.
    counts, count_freqs = np.array([1, 2, 3, 4, 5]), np.array([63, 15, 2, 3, 1])
    count_p = count_freqs / count_freqs.sum()

    n_unreachable = max(1, round(n_outcomes * 4 / 88))
    n_unconditional = max(1, round(n_outcomes * 2 / 88))
    order = rng.permutation(n_outcomes)
    unreachable = set(order[:n_unreachable].tolist())
    unconditional = set(order[n_unreachable:n_unreachable + n_unconditional].tolist())

    arms: list[tuple[int, tuple[int, ...]]] = []
    for outcome in range(n_outcomes):
        if outcome in unreachable:
            continue
        if outcome in unconditional:
            arms.append((outcome, ()))
            continue
        for _ in range(int(rng.choice(counts, p=count_p))):
            size = min(int(rng.choice(sizes, p=size_p)), n_inputs)
            members = rng.choice(n_inputs, size=size, replace=False, p=weights)
            arms.append((outcome, tuple(sorted(int(m) for m in members))))

    # A is the OR-flattening of the arms, which the Instance invariant requires.
    A = np.zeros((n_inputs, n_outcomes), dtype=np.uint8)
    for outcome, members in arms:
        for member in members:
            A[member, outcome] = 1

    return Instance(
        instance_id=instance_id,
        regime="conjunctive_panel",
        seed=seed,
        A=A,
        w=make_weights(A, weight_scheme, rng),
        c=make_costs(n_inputs, rng),
        arms=arms,
        notes=("SYNTHETIC conjunctive instance, shape-calibrated to "
               "QBMED-HEME-001. Contains no clinical content and supports no "
               "clinical claim."),
    )


def heme_benchmark(seed: int = 42, weight_scheme: str = "uniform",
                   instance_id: str = "QBMED-HEME-001") -> Instance:
    """The flagship 66-input / 88-outcome benchmark instance (spec section 3).

    Loads the real Q-BenchMed-Heme domain profile and projects it onto the core
    `Instance` type.  The import is local because the core must not depend on
    the domain layer (spec section 24): `qbm.instances` is domain-independent
    and this one convenience constructor is the only place the flagship domain
    is named.

    `weight_scheme` other than "uniform" re-derives outcome weights from the
    incidence structure, which makes the instance a *variant* of the published
    benchmark rather than the benchmark itself; the instance_id is suffixed so
    the two can never be confused in a results table.
    """
    # Local import: the core must not depend on the domain layer, so the one
    # place a domain is named is here. See HemeDomainUnavailable below.
    try:
        from qbm.domains import heme as _heme
    except ImportError as error:  # pragma: no cover - depends on the checkout
        raise HemeDomainUnavailable(
            "qbm.domains.heme could not be imported, so QBMED-HEME-001 is "
            "unavailable in this checkout. It ships with the package, so this "
            "usually means a partial install.\n\n"
            "qbm.instances.conjunctive_panel() gives a synthetic instance with "
            "the same structural properties if you need one now. The published "
            "results computed on QBMED-HEME-001 are in "
            "reproducibility/REPORT.md."
        ) from error

    inst = _heme.build_profile().to_instance(instance_id=instance_id)
    if weight_scheme != "uniform":
        rng = np.random.default_rng(seed)
        inst = Instance(
            instance_id=f"{instance_id}-{weight_scheme}",
            regime=inst.regime, seed=seed, A=inst.A,
            w=make_weights(inst.A, weight_scheme, rng), c=inst.c,
            weight_scheme=weight_scheme,
            input_names=inst.input_names, outcome_names=inst.outcome_names,
            notes=inst.notes + f" | outcome weights re-derived: {weight_scheme}",
            arms=inst.arms,
        )
    return inst


def synthetic_heme_shaped(seed: int = 42, weight_scheme: str = "uniform",
                          instance_id: str = "SYNTH-HEME-SHAPED") -> Instance:
    """Panel-shaped synthetic instance -- the former stand-in, kept for scaling.

    Contains no clinical knowledge and supports no clinical claim.  It exists
    so degree structure can be varied freely without touching the real profile.
    """
    inst = generate("heme_shaped", 66, 88, seed=seed,
                    weight_scheme=weight_scheme, instance_id=instance_id)
    inst.notes = ("SYNTHETIC panel-shaped instance; imitates biomarker degree "
                  "structure only. Not the Q-BenchMed-Heme benchmark.")
    return inst


def scaling_ladder(base: Instance, sizes=(10, 20, 30, 40, 50, 66)) -> list["Instance"]:
    """Nested sub-instances of increasing size (spec section 18)."""
    return [base.subinstance(n, instance_id=f"{base.instance_id}-n{n}")
            for n in sizes if n <= base.n_inputs]


def regime_suite(n_inputs: int, n_outcomes: int, seed: int,
                 weight_scheme: str = "uniform") -> dict[str, "Instance"]:
    """One instance per synthetic structure regime at a fixed size."""
    return {
        r: generate(r, n_inputs, n_outcomes, seed=seed, weight_scheme=weight_scheme)
        for r in REGIMES if r != "heme_shaped"
    }
