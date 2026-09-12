"""Domain-profile layer: the adapter boundary of spec section 26.

A *profile* is a declarative, on-disk description of one biomedical domain's
optimization problem.  It is the only place where domain knowledge lives.  The
core (`qbm.qubo`, `qbm.qaoa`, `qbm.classical`) never imports this module; the
data flows one way:

    profile on disk  ->  DomainProfile  ->  Instance  ->  Qubo  ->  solvers

so a new domain is added by writing a profile directory, not by touching the
optimization engine (spec section 24).

Typed relationships
-------------------
Spec section 21 rejects reducing every biomedical relationship to a binary
value.  Five edge types are supported:

    required      outcome cannot be covered unless this input is selected
    supporting    input contributes to the outcome (the classical binary edge)
    optional      contributes, but carries reduced credit
    exclusionary  selecting this input *blocks* the outcome
    contextual    contributes only when its context input is also selected

Path groups
-----------
Real decision rules are disjunctions of conjunctions: an outcome fires when
*any one* of its authored rule arms has *all* of its inputs available.  Each
edge therefore carries a `path` label, and edges sharing a path form a
conjunction; the outcome needs only one path to be satisfied.

    path "A": {HGB, MCV, FERRITIN}   -- arm A of the rule
    path "B": {MCV, HGB, RDW_CV}     -- arm B of the rule
    covered  <=>  A satisfied OR B satisfied

An edge with no path label is its own singleton path, which is why the binary
case falls out unchanged.

Coverage semantics
------------------
Outcome j is covered by selection x iff

    j is unconditional, OR

    (1) every unpathed `required` input of j is in x, AND
    (2) no unpathed `exclusionary` input of j is in x, AND
    (3) at least one path group of j is satisfied

where a path group is satisfied when every non-exclusionary member is in x, no
exclusionary member is in x, and every `contextual` member's context input is
also in x.  The unpathed `required` edges of an outcome form a group of their
own, so `required` also contributes rather than only gating.

This definition is deliberately backward-compatible: when every edge is
`supporting` with no path label, (1) and (2) are vacuous, every edge is its own
singleton group, and coverage reduces to "at least one selected input touches
j" -- exactly the binary incidence semantics of `qbm.instances.Instance`.
Existing results are therefore unchanged, which is what makes the typed model
safe to introduce (spec section 22: begin with a transparent model before
adding coefficients).

Conditions (1)-(3) are also what make the objective non-submodular: adding an
input can *remove* coverage.  Greedy's 1-1/e guarantee does not apply to such
instances.

Unconditional outcomes
----------------------
A real rule set can contain a priority-0 fallback that fires when nothing else
matches, so it is reachable from the empty selection.  Such outcomes are named
in `unconditional_outcomes` and are always covered.  They are not an error, but
they inflate any coverage ratio, so `benchmark_metadata()` reports their count
separately and the benchmark layer can exclude them.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import yaml

from .instances import Instance

SCHEMA_VERSION = "qbm-profile/1.0"

# Edge types, in the order they are documented in spec section 21.
REL_TYPES = ("required", "supporting", "optional", "exclusionary", "contextual")

# Types that grant coverage credit when active.
CONTRIBUTING = frozenset({"required", "supporting", "optional", "contextual"})

# Confidence classifications a profile may declare per edge.  These are
# provenance labels, not probabilities -- the framework does not interpret them
# numerically, it only requires that one be declared (spec section 21).
CONFIDENCE_CLASSES = ("defined", "derived", "assumed")

# Default credit multiplier by type.  `optional` edges carry less credit than a
# full supporting edge; the rest carry full credit when active.
DEFAULT_CREDIT = {
    "required": 1.0,
    "supporting": 1.0,
    "optional": 0.5,
    "contextual": 1.0,
    "exclusionary": 0.0,
}


class ProfileError(ValueError):
    """Raised only for structural damage that prevents a profile being read.

    Content problems are *collected* by `validate()` and returned, not raised:
    a domain expert fixing an export needs the whole list, not the first error.
    """


# --------------------------------------------------------------------------
# edge
# --------------------------------------------------------------------------
@dataclass(frozen=True)
class Relationship:
    """One typed, provenanced input -> outcome edge (spec section 21)."""

    input_id: str
    outcome_id: str
    rel_type: str
    weight: float = 1.0
    context_input_id: str | None = None
    source: str = ""
    confidence: str = "defined"
    # Rule arm this edge belongs to.  Empty means "own singleton path" for
    # supporting/optional edges, or "applies to every path" for required and
    # exclusionary edges.
    path: str = ""

    def credit(self) -> float:
        """Coverage credit this edge grants when active."""
        return DEFAULT_CREDIT[self.rel_type] * self.weight


# --------------------------------------------------------------------------
# profile
# --------------------------------------------------------------------------
@dataclass
class DomainProfile:
    """A biomedical domain's problem definition (spec section 26 interface).

    The six accessors named in the spec -- inputs, outcomes, relationships,
    constraints, objective, benchmark_metadata -- are the whole public surface
    the optimization engine is allowed to depend on.
    """

    benchmark_id: str
    domain: str
    input_ids: list[str]
    outcome_ids: list[str]
    edges: list[Relationship]
    outcome_weights: dict[str, float] = field(default_factory=dict)
    input_costs: dict[str, float] = field(default_factory=dict)
    constraint_spec: dict = field(default_factory=dict)
    objective_spec: dict = field(default_factory=dict)
    provenance: dict = field(default_factory=dict)
    notes: str = ""
    # Outcomes reachable from the empty selection (priority-0 fallbacks in a
    # real rule set).  Always covered; counted separately so they cannot
    # silently inflate a coverage ratio.
    unconditional_outcomes: list[str] = field(default_factory=list)

    # ---- spec section 26 interface ---------------------------------------
    def inputs(self) -> list[str]:
        return list(self.input_ids)

    def outcomes(self) -> list[str]:
        return list(self.outcome_ids)

    def relationships(self) -> list[Relationship]:
        return list(self.edges)

    def constraints(self) -> dict:
        return dict(self.constraint_spec)

    def objective(self) -> dict:
        return dict(self.objective_spec)

    def benchmark_metadata(self) -> dict:
        n_by_type = {t: sum(1 for e in self.edges if e.rel_type == t) for t in REL_TYPES}
        return {
            "schema_version": SCHEMA_VERSION,
            "benchmark_id": self.benchmark_id,
            "domain": self.domain,
            "n_inputs": len(self.input_ids),
            "n_outcomes": len(self.outcome_ids),
            "n_edges": len(self.edges),
            "edges_by_type": n_by_type,
            "n_paths": sum(len(g) for g in self.path_groups().values()),
            "n_unconditional_outcomes": len(self.unconditional_outcomes),
            "n_reachable_outcomes": int(self.reachable_outcomes().sum()),
            "n_inert_inputs": int((self.incidence().sum(axis=1) == 0).sum()),
            "is_binary_equivalent": self.is_binary_equivalent(),
            "checksum": self.checksum(),
            # Provenance flags travel with the profile so that a downstream
            # report can tell a real export from a stand-in without guessing.
            "real_export_present": bool(self.provenance.get("real_export_present", False)),
            "synthetic_stand_in": bool(self.provenance.get("synthetic_stand_in", False)),
            "knowledge_version": self.provenance.get("knowledge_version", "unspecified"),
            "notes": self.notes,
        }

    # ---- structure -------------------------------------------------------
    def is_binary_equivalent(self) -> bool:
        """True when coverage reduces to plain binary incidence.

        Used by tests and by the QUBO builder to assert that introducing the
        typed model did not change any pre-existing instance.
        """
        return (not self.unconditional_outcomes) and all(
            e.rel_type == "supporting" and e.weight == 1.0 and not e.path
            for e in self.edges
        )

    def incidence(self) -> np.ndarray:
        """(n_inputs, n_outcomes) uint8 matrix of *contributing* edges.

        Exclusionary edges are excluded here by construction; they are carried
        separately by `exclusion_matrix` because they act with opposite sign.
        """
        idx_i = {k: i for i, k in enumerate(self.input_ids)}
        idx_o = {k: j for j, k in enumerate(self.outcome_ids)}
        A = np.zeros((len(self.input_ids), len(self.outcome_ids)), dtype=np.uint8)
        for e in self.edges:
            if e.rel_type in CONTRIBUTING:
                A[idx_i[e.input_id], idx_o[e.outcome_id]] = 1
        return A

    def exclusion_matrix(self) -> np.ndarray:
        idx_i = {k: i for i, k in enumerate(self.input_ids)}
        idx_o = {k: j for j, k in enumerate(self.outcome_ids)}
        X = np.zeros((len(self.input_ids), len(self.outcome_ids)), dtype=np.uint8)
        for e in self.edges:
            if e.rel_type == "exclusionary":
                X[idx_i[e.input_id], idx_o[e.outcome_id]] = 1
        return X

    def required_matrix(self) -> np.ndarray:
        idx_i = {k: i for i, k in enumerate(self.input_ids)}
        idx_o = {k: j for j, k in enumerate(self.outcome_ids)}
        R = np.zeros((len(self.input_ids), len(self.outcome_ids)), dtype=np.uint8)
        for e in self.edges:
            if e.rel_type == "required":
                R[idx_i[e.input_id], idx_o[e.outcome_id]] = 1
        return R

    def context_pairs(self) -> list[tuple[int, int, int]]:
        """(input_idx, outcome_idx, context_input_idx) for contextual edges."""
        idx_i = {k: i for i, k in enumerate(self.input_ids)}
        idx_o = {k: j for j, k in enumerate(self.outcome_ids)}
        out = []
        for e in self.edges:
            if e.rel_type == "contextual" and e.context_input_id:
                out.append((idx_i[e.input_id], idx_o[e.outcome_id],
                            idx_i[e.context_input_id]))
        return out

    def path_groups(self) -> dict[str, list[list[Relationship]]]:
        """outcome_id -> list of path groups, each a conjunction of edges.

        Unpathed supporting/optional edges become singleton groups, which is
        what makes the binary case reduce to "any selected input touches j".
        Unpathed exclusionary edges are not groups -- they are global vetoes
        handled by `covered`.  Unpathed required edges are both a global
        conjunct and, taken together, one group, so an outcome whose only edge
        is required is covered once that input is selected.
        """
        groups: dict[str, dict[str, list[Relationship]]] = {}
        req: dict[str, list[Relationship]] = {}
        for k, e in enumerate(self.edges):
            if not e.path and e.rel_type == "exclusionary":
                continue
            if not e.path and e.rel_type == "required":
                # Also a global conjunct (see `covered`), but a required edge
                # contributes as well: an outcome whose only edge is `required`
                # is covered once that input is selected.
                req.setdefault(e.outcome_id, []).append(e)
                continue
            key = e.path if e.path else f"\x00singleton{k}"
            groups.setdefault(e.outcome_id, {}).setdefault(key, []).append(e)
        out = {o: list(g.values()) for o, g in groups.items()}
        for o, edges in req.items():
            out.setdefault(o, []).append(edges)
        return out

    # ---- coverage --------------------------------------------------------
    def covered(self, x: np.ndarray) -> np.ndarray:
        """Boolean (n_outcomes,) mask under the typed semantics documented above."""
        x = np.asarray(x, dtype=bool)
        idx_i = {k: i for i, k in enumerate(self.input_ids)}
        sel = {self.input_ids[i] for i in np.flatnonzero(x)}

        # Global conjuncts: unpathed required must all be present, unpathed
        # exclusionary must all be absent.
        glob_req: dict[str, set[str]] = {}
        glob_excl: dict[str, set[str]] = {}
        for e in self.edges:
            if e.path:
                continue
            if e.rel_type == "required":
                glob_req.setdefault(e.outcome_id, set()).add(e.input_id)
            elif e.rel_type == "exclusionary":
                glob_excl.setdefault(e.outcome_id, set()).add(e.input_id)

        groups = self.path_groups()
        uncond = set(self.unconditional_outcomes)
        out = np.zeros(len(self.outcome_ids), dtype=bool)

        for j, o in enumerate(self.outcome_ids):
            if o in uncond:
                out[j] = True
                continue
            if not glob_req.get(o, set()) <= sel:
                continue
            if glob_excl.get(o, set()) & sel:
                continue
            for grp in groups.get(o, ()):
                ok = True
                for e in grp:
                    present = e.input_id in sel
                    if e.rel_type == "exclusionary":
                        if present:
                            ok = False
                            break
                        continue
                    if not present:
                        ok = False
                        break
                    if e.rel_type == "contextual" and e.context_input_id:
                        if e.context_input_id not in sel:
                            ok = False
                            break
                if ok:
                    out[j] = True
                    break
        return out

    def reachable_outcomes(self) -> np.ndarray:
        """Outcomes coverable by *some* selection (all inputs on, and empty)."""
        allon = self.covered(np.ones(len(self.input_ids), dtype=bool))
        # An exclusionary-guarded outcome can be unreachable with everything on
        # yet reachable with a subset, so also credit any outcome with a group
        # whose non-exclusionary members exist in the input list.
        groups = self.path_groups()
        for j, o in enumerate(self.outcome_ids):
            if allon[j] or o in set(self.unconditional_outcomes):
                allon[j] = True
                continue
            for grp in groups.get(o, ()):
                if all(e.input_id in self.input_ids for e in grp
                       if e.rel_type != "exclusionary"):
                    allon[j] = True
                    break
        return allon

    def coverage_weight(self, x: np.ndarray) -> float:
        w = self.weight_vector()
        return float(w[self.covered(x)].sum())

    def weight_vector(self) -> np.ndarray:
        return np.array([self.outcome_weights.get(o, 1.0) for o in self.outcome_ids],
                        dtype=float)

    def cost_vector(self) -> np.ndarray:
        return np.array([self.input_costs.get(i, 1.0) for i in self.input_ids],
                        dtype=float)

    # ---- adapter ---------------------------------------------------------
    def to_instance(self, instance_id: str | None = None) -> Instance:
        """Project onto the core `Instance` type.

        This is the adapter boundary.  For a binary-equivalent profile the
        projection is lossless.  When the profile uses required/exclusionary/
        contextual edges the projection keeps only the contributing incidence,
        so callers that need the full semantics must use `covered()` or the
        typed QUBO builder -- `to_instance` reports the loss in `notes`.
        """
        arms, lossy_reasons = self._arms_for_instance()
        note = self.notes
        if lossy_reasons:
            note = (note + " | PROJECTION LOSSY: " + "; ".join(lossy_reasons) +
                    "; use DomainProfile.covered() for exact semantics").strip(" |")
        return Instance(
            instance_id=instance_id or self.benchmark_id,
            regime="profile",
            seed=int(self.provenance.get("seed", -1)),
            A=self.incidence(),
            w=self.weight_vector(),
            c=self.cost_vector(),
            weight_scheme=str(self.objective_spec.get("outcome_weighting", "uniform")),
            input_names=list(self.input_ids),
            outcome_names=list(self.outcome_ids),
            notes=note,
            arms=arms,
        )

    def _arms_for_instance(self) -> tuple[list[tuple[int, tuple[int, ...]]], list[str]]:
        """Translate the typed semantics into monotone-DNF arms.

        `Instance` coverage is a disjunction of conjunctions over *selected*
        inputs, so it can represent required edges (conjuncts) and alternative
        paths (disjuncts) exactly.  It cannot represent an edge whose effect is
        triggered by *absence*: an exclusionary edge makes coverage
        non-monotone, which no positive DNF can express.  Rather than silently
        drop such an edge, the arm it guards is omitted and the loss is named in
        the returned reasons, so a caller reading `Instance.notes` learns that
        `covered()` is an upper bound on the true semantics.
        """
        idx_i = {k: i for i, k in enumerate(self.input_ids)}
        reasons: list[str] = []

        glob_req: dict[str, set[str]] = {}
        glob_excl: dict[str, set[str]] = {}
        for e in self.edges:
            if e.path:
                continue
            if e.rel_type == "required":
                glob_req.setdefault(e.outcome_id, set()).add(e.input_id)
            elif e.rel_type == "exclusionary":
                glob_excl.setdefault(e.outcome_id, set()).add(e.input_id)

        groups = self.path_groups()
        uncond = set(self.unconditional_outcomes)
        arms: list[tuple[int, tuple[int, ...]]] = []

        for j, o in enumerate(self.outcome_ids):
            if o in uncond:
                # The engine emits this outcome even with nothing supplied, so
                # the empty arm is satisfied unconditionally.  Its authored arms
                # are then redundant but are still emitted: dropping them would
                # leave edges of A unexplained, and the invariant that A is the
                # OR-flattening of the arms is what keeps the two views of the
                # instance describing one problem.
                arms.append((j, ()))
            if glob_excl.get(o):
                reasons.append(f"{o}: exclusionary guard dropped")
            base = set(glob_req.get(o, ()))
            for grp in groups.get(o, ()):
                members = set(base)
                blocked = False
                for e in grp:
                    if e.rel_type == "exclusionary":
                        blocked = True
                        break
                    members.add(e.input_id)
                    if e.rel_type == "contextual" and e.context_input_id:
                        members.add(e.context_input_id)
                if blocked:
                    reasons.append(f"{o}: arm with exclusionary member dropped")
                    continue
                arms.append((j, tuple(sorted(idx_i[m] for m in members))))

        return arms, sorted(set(reasons))

    # ---- provenance ------------------------------------------------------
    def checksum(self) -> str:
        h = hashlib.sha256()
        h.update(self.benchmark_id.encode())
        for e in sorted(self.edges,
                        key=lambda e: (e.input_id, e.outcome_id, e.rel_type, e.path)):
            h.update(f"{e.input_id}|{e.outcome_id}|{e.rel_type}|{e.weight:.10g}|"
                     f"{e.context_input_id or ''}|{e.path}".encode())
        for o in self.outcome_ids:
            h.update(f"{o}:{self.outcome_weights.get(o, 1.0):.10g}".encode())
        for i in self.input_ids:
            h.update(f"{i}:{self.input_costs.get(i, 1.0):.10g}".encode())
        for o in sorted(self.unconditional_outcomes):
            h.update(f"uncond:{o}".encode())
        return h.hexdigest()[:16]

    # ---- serialization ---------------------------------------------------
    def save(self, directory: str | Path) -> Path:
        d = Path(directory)
        d.mkdir(parents=True, exist_ok=True)
        header = {
            "schema_version": SCHEMA_VERSION,
            "benchmark_id": self.benchmark_id,
            "domain": self.domain,
            "inputs": [{"id": i, "cost": self.input_costs.get(i, 1.0)}
                       for i in self.input_ids],
            "outcomes": [{"id": o, "weight": self.outcome_weights.get(o, 1.0)}
                         for o in self.outcome_ids],
            "constraints": self.constraint_spec,
            "objective": self.objective_spec,
            "provenance": self.provenance,
            "unconditional_outcomes": list(self.unconditional_outcomes),
            "notes": self.notes,
            "checksum": self.checksum(),
        }
        (d / "profile.yaml").write_text(yaml.safe_dump(header, sort_keys=False))
        lines = ["input_id,outcome_id,rel_type,weight,context_input_id,path,"
                 "confidence,source"]
        for e in self.edges:
            lines.append(",".join([
                e.input_id, e.outcome_id, e.rel_type, f"{e.weight:.17g}",
                e.context_input_id or "", e.path, e.confidence,
                e.source.replace(",", ";"),
            ]))
        (d / "relationships.csv").write_text("\n".join(lines) + "\n")
        return d

    @classmethod
    def load(cls, directory: str | Path, verify_checksum: bool = True) -> "DomainProfile":
        d = Path(directory)
        py, rc = d / "profile.yaml", d / "relationships.csv"
        for f in (py, rc):
            if not f.exists():
                raise ProfileError(f"profile directory {d} is missing {f.name}")
        header = yaml.safe_load(py.read_text())
        if not isinstance(header, dict):
            raise ProfileError(f"{py} did not parse to a mapping")

        rows = [r for r in rc.read_text().splitlines() if r.strip()]
        if not rows:
            raise ProfileError(f"{rc} is empty")
        cols = [c.strip() for c in rows[0].split(",")]
        edges = []
        for ln, raw in enumerate(rows[1:], start=2):
            parts = raw.split(",")
            if len(parts) != len(cols):
                raise ProfileError(
                    f"{rc}:{ln} has {len(parts)} fields, header declares {len(cols)}")
            rec = dict(zip(cols, (p.strip() for p in parts)))
            edges.append(Relationship(
                input_id=rec["input_id"],
                outcome_id=rec["outcome_id"],
                rel_type=rec["rel_type"],
                weight=float(rec.get("weight") or 1.0),
                context_input_id=rec.get("context_input_id") or None,
                source=rec.get("source", ""),
                confidence=rec.get("confidence") or "defined",
                path=rec.get("path", ""),
            ))

        prof = cls(
            benchmark_id=header.get("benchmark_id", d.name),
            domain=header.get("domain", "unspecified"),
            input_ids=[i["id"] for i in header.get("inputs", [])],
            outcome_ids=[o["id"] for o in header.get("outcomes", [])],
            edges=edges,
            outcome_weights={o["id"]: float(o.get("weight", 1.0))
                             for o in header.get("outcomes", [])},
            input_costs={i["id"]: float(i.get("cost", 1.0))
                         for i in header.get("inputs", [])},
            constraint_spec=header.get("constraints") or {},
            objective_spec=header.get("objective") or {},
            provenance=header.get("provenance") or {},
            notes=header.get("notes") or "",
            unconditional_outcomes=list(header.get("unconditional_outcomes") or []),
        )
        declared = header.get("checksum")
        if verify_checksum and declared and declared != prof.checksum():
            raise ProfileError(
                f"checksum mismatch for {d.name}: profile.yaml declares {declared} "
                f"but the loaded content hashes to {prof.checksum()}. The profile "
                f"was edited without updating its checksum, or a file is corrupt.")
        return prof


# --------------------------------------------------------------------------
# validation
# --------------------------------------------------------------------------
def validate(profile: DomainProfile, *, expect_inputs: int | None = None,
             expect_outcomes: int | None = None) -> list[str]:
    """Return every problem found, as actionable messages. Empty list == valid.

    Errors are collected rather than raised: someone repairing a 5808-edge
    export needs the full list in one pass (spec section 21).
    """
    errs: list[str] = []
    ins, outs = set(profile.input_ids), set(profile.outcome_ids)

    if len(profile.input_ids) != len(ins):
        dupes = _dupes(profile.input_ids)
        errs.append(f"duplicate input ids: {sorted(dupes)[:10]}")
    if len(profile.outcome_ids) != len(outs):
        dupes = _dupes(profile.outcome_ids)
        errs.append(f"duplicate outcome ids: {sorted(dupes)[:10]}")
    if not profile.input_ids:
        errs.append("profile declares no inputs")
    if not profile.outcome_ids:
        errs.append("profile declares no outcomes")

    seen: set[tuple[str, str, str]] = set()
    for n, e in enumerate(profile.edges, start=1):
        where = f"relationships.csv row {n} ({e.input_id} -> {e.outcome_id})"
        if e.input_id not in ins:
            errs.append(f"{where}: unknown input id '{e.input_id}' "
                        f"-- not declared in profile.yaml inputs")
        if e.outcome_id not in outs:
            errs.append(f"{where}: unknown outcome id '{e.outcome_id}' "
                        f"-- not declared in profile.yaml outcomes")
        if e.rel_type not in REL_TYPES:
            errs.append(f"{where}: rel_type '{e.rel_type}' is not one of {REL_TYPES}")
        if not np.isfinite(e.weight) or e.weight < 0:
            errs.append(f"{where}: weight {e.weight!r} must be finite and >= 0")
        if e.rel_type == "contextual" and not e.context_input_id:
            errs.append(f"{where}: contextual edge needs a context_input_id")
        if e.context_input_id and e.context_input_id not in ins:
            errs.append(f"{where}: unknown context_input_id '{e.context_input_id}'")
        if e.context_input_id == e.input_id:
            errs.append(f"{where}: contextual edge cannot be its own context")
        if e.confidence not in CONFIDENCE_CLASSES:
            errs.append(f"{where}: confidence '{e.confidence}' not in {CONFIDENCE_CLASSES}")
        if not e.source:
            errs.append(f"{where}: missing source -- every edge needs provenance "
                        f"(spec section 21)")
        key = (e.input_id, e.outcome_id, e.rel_type, e.path)
        if key in seen:
            errs.append(f"{where}: duplicate edge, already declared")
        seen.add(key)

    for o in profile.unconditional_outcomes:
        if o not in outs:
            errs.append(f"unconditional_outcomes names '{o}', which is not a "
                        f"declared outcome")

    for name, got, want in (("inputs", len(profile.input_ids), expect_inputs),
                            ("outcomes", len(profile.outcome_ids), expect_outcomes)):
        if want is not None and got != want:
            errs.append(f"expected {want} {name}, profile declares {got}")

    for o, wv in profile.outcome_weights.items():
        if not np.isfinite(wv) or wv <= 0:
            errs.append(f"outcome '{o}': weight {wv!r} must be finite and > 0")
    for i, cv in profile.input_costs.items():
        if not np.isfinite(cv) or cv <= 0:
            errs.append(f"input '{i}': cost {cv!r} must be finite and > 0")

    return errs


def structural_findings(profile: DomainProfile) -> list[str]:
    """Report structural properties that change how results must be read.

    These are deliberately *not* validation errors.  A real knowledge export
    can legitimately declare an input no active rule consumes, or an outcome no
    input can reach; refusing to load such a profile would make the real
    benchmark unusable and would hide the property instead of recording it.
    But each one changes the interpretation of a coverage number, so the
    benchmark layer must surface them alongside every result.
    """
    out: list[str] = []
    A = profile.incidence()
    reach = profile.reachable_outcomes()

    unreachable = [profile.outcome_ids[j] for j in np.flatnonzero(~reach)]
    if unreachable:
        out.append(
            f"{len(unreachable)}/{len(profile.outcome_ids)} outcome(s) are "
            f"unreachable by any authored path, so weighted coverage can never "
            f"reach 100%; optimality gaps must be taken against the achievable "
            f"maximum, not the declared outcome count: {unreachable}")

    inert = [profile.input_ids[i] for i in np.flatnonzero(A.sum(axis=1) == 0)]
    if inert:
        out.append(
            f"{len(inert)}/{len(profile.input_ids)} input(s) contribute to no "
            f"outcome. They can only ever add cost, so the effective decision "
            f"dimension is {len(profile.input_ids) - len(inert)}, not "
            f"{len(profile.input_ids)}: {inert}")

    if profile.unconditional_outcomes:
        out.append(
            f"{len(profile.unconditional_outcomes)} outcome(s) are covered by "
            f"the empty selection and so contribute a constant to every "
            f"objective value: {list(profile.unconditional_outcomes)}")

    n_excl = sum(1 for e in profile.edges if e.rel_type == "exclusionary")
    if n_excl:
        out.append(
            f"{n_excl} exclusionary edge(s) present: the coverage objective is "
            f"not submodular, so greedy carries no 1-1/e guarantee on this "
            f"profile")

    multi = {o: len(g) for o, g in profile.path_groups().items() if len(g) > 1}
    if multi:
        out.append(
            f"{len(multi)} outcome(s) have alternative rule arms (max "
            f"{max(multi.values())} paths); coverage is a disjunction of "
            f"conjunctions, not simple incidence")

    return out


def _dupes(seq) -> set:
    seen, out = set(), set()
    for s in seq:
        if s in seen:
            out.add(s)
        seen.add(s)
    return out
