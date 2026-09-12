"""Benchmark instance generation for Q-BenchMed.

An instance is a bipartite incidence structure between *inputs* (the binary
decision variables) and *outcomes* (the targets to be covered), plus outcome
weights and input costs.

    A[i, j] = 1  <=>  input i contributes to outcome j

The generator is domain-agnostic by design (spec section 24): nothing here knows
that hematology exists.  A domain adapter supplies A, w, c and metadata; the
`heme_shaped` regime below is a *synthetic stand-in* whose degree structure
imitates a laboratory panel, used until the real 66x88 export is available.

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
        """Boolean mask of outcomes covered by input selection x."""
        x = np.asarray(x, dtype=bool)
        if not x.any():
            return np.zeros(self.n_outcomes, bool)
        return self.A[x].sum(axis=0) > 0

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
        (d / "metadata.json").write_text(json.dumps(self.metadata(), indent=2) + "\n")
        return d

    @classmethod
    def load(cls, directory: str | Path) -> "Instance":
        d = Path(directory)
        meta = json.loads((d / "metadata.json").read_text())
        A = np.loadtxt(d / "incidence.csv", delimiter=",", dtype=np.uint8, ndmin=2)
        w = np.loadtxt(d / "outcome_weights.csv", delimiter=",", ndmin=1)
        c = np.loadtxt(d / "input_costs.csv", delimiter=",", ndmin=1)
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
    """Incidence structure imitating a laboratory-panel knowledge base.

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


def heme_benchmark(seed: int = 42, weight_scheme: str = "uniform",
                   instance_id: str = "QBMED-HEME-001") -> Instance:
    """The flagship 66-input / 88-outcome benchmark instance (spec section 3)."""
    inst = generate("heme_shaped", 66, 88, seed=seed,
                    weight_scheme=weight_scheme, instance_id=instance_id)
    inst.notes = (
        "SYNTHETIC stand-in for the 66x88 pathology knowledge export. "
        "Degree structure imitates a laboratory panel; contains no clinical "
        "knowledge and supports no clinical claim. Replace incidence.csv with "
        "the real export to run the true benchmark."
    )
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
