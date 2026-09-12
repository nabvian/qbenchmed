"""Versioned result record -- the single output format of every algorithm.

Spec section 63 asks for a common result object and section 42 lists what a
benchmark execution must record.  This module is the union of the two, with the
schema version carried *inside* every record so a stored result can never be
misread by a later reader:

    qbm-result/MAJOR.MINOR.PATCH

    MAJOR  a field changed meaning, or a required field was removed.  A reader
           for an older major version must refuse the record.
    MINOR  a field was added.  Older readers can ignore it.
    PATCH  documentation or defaults only.

`SCHEMA_VERSION` is the version this build writes; `read_jsonl` refuses a file
whose major version it does not know rather than silently coercing it.

    1.1.0  added the QUBO-energy axis (`qubo_energy`, `qubo_gap_*`) and gave
           `noise_model` a dict payload.  `noise_model` was declared in 1.0.0
           but never written by any 1.0.0 producer, so no stored record can be
           misread by the type change and the bump stays MINOR.

Three design commitments are worth stating because they are what make stored
records comparable rather than merely present.

**Runtime is decomposed, never a single number** (spec section 44).  A QAOA run
on a simulator spends its time in circuit simulation; a greedy run spends it in
marginal-gain evaluation.  Reporting one total invites the comparison the
project explicitly refuses to make, so `timings_ms` keeps the phases apart and
`runtime_ms` is documented as the algorithm phase alone.

**Simulated quantum time is not quantum time.**  `wall_clock_comparable` is
False for every record produced by a statevector backend.  Spec section 50
forbids conflating simulator wall clock with QPU wall clock, and a boolean in
the record is harder to overlook than a sentence in a report.

**Gaps are measured against a certified reference or not at all.**
`optimality_gap_*` stays None unless some algorithm in the same comparison
returned `certified_optimal`; `reference_source` names which one.

Two gap axes are recorded, and they answer different questions.  The *native*
gap asks how good the selected input panel is; it is defined only for feasible
answers, because the native objective carries no budget penalty and an
over-budget selection would otherwise score better than the constrained
optimum.  The *QUBO* gap asks how well the solver minimised the function it was
actually handed, penalties included; it needs no feasibility filter, since a
constraint violation is already priced into the energy.  A record can have a
QUBO gap and no native gap -- that combination means the solver found a
low-energy state that does not decode to a feasible panel, which is exactly the
failure mode a penalty formulation is prone to and should stay visible.
"""

from __future__ import annotations

import hashlib
import json
import platform
import subprocess
import sys
from dataclasses import asdict, dataclass, field, fields
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

SCHEMA_VERSION = "qbm-result/1.1.0"
SCHEMA_MAJOR = 1

# Fields whose values identify *what was run*, as opposed to what came out.
# The record id is their hash, so re-running an identical configuration
# produces the same id and a results file can be deduplicated mechanically.
IDENTITY_FIELDS = (
    "benchmark_id", "instance_id", "instance_checksum", "objective",
    "input_budget", "coverage_target", "weighting", "cost_model",
    "formulation", "encoding", "algorithm", "algorithm_version",
    "algorithm_params", "seed", "qaoa_depth", "shots", "noise_model",
)


def _git_commit() -> str | None:
    try:
        out = subprocess.run(["git", "rev-parse", "HEAD"], capture_output=True,
                             text=True, timeout=5, cwd=Path(__file__).parent)
        if out.returncode == 0:
            return out.stdout.strip()
    except Exception:
        pass
    return None


_ENV_CACHE: dict | None = None


def environment_fingerprint() -> dict:
    """Software and hardware identification for the reproducibility block.

    Deliberately *not* a full ``pip freeze``: only the packages whose version
    can change a numerical result are recorded, so a diff between two records
    points at something that matters.  CPU count is recorded because runtime
    metrics are meaningless without it; no hostname or user is recorded, since
    a public benchmark artifact should not carry them.
    """
    global _ENV_CACHE
    if _ENV_CACHE is not None:
        return dict(_ENV_CACHE)
    versions = {}
    for mod in ("numpy", "scipy", "pandas", "matplotlib"):
        try:
            versions[mod] = __import__(mod).__version__
        except Exception:
            versions[mod] = None
    try:
        import os
        cpus = os.cpu_count()
    except Exception:
        cpus = None
    _ENV_CACHE = {
        "python": sys.version.split()[0],
        "implementation": platform.python_implementation(),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "cpu_count": cpus,
        "packages": versions,
        "git_commit": _git_commit(),
    }
    return dict(_ENV_CACHE)


@dataclass
class RunRecord:
    """One algorithm, one instance, one seed -- everything needed to reproduce it."""

    # ---- identity ---------------------------------------------------------
    benchmark_id: str
    instance_id: str
    instance_checksum: str
    algorithm: str
    algorithm_version: str

    # ---- problem ----------------------------------------------------------
    benchmark_version: str = "unversioned"
    regime: str = ""
    n_inputs: int = 0
    n_outcomes: int = 0
    n_arms: int = 0
    is_conjunctive: bool = False
    n_unconditional_outcomes: int = 0
    n_reachable_outcomes: int = 0

    # ---- formulation ------------------------------------------------------
    objective: str = "maximum_outcome_coverage"
    input_budget: int | None = None
    coverage_target: float | None = None
    weighting: str = "uniform"
    cost_model: str = "uniform"
    formulation: str = "native"          # "native" | "qubo"
    encoding: str | None = None
    penalties: dict = field(default_factory=dict)   # alpha, beta, gamma, lam
    n_problem_variables: int = 0

    # ---- algorithm configuration -----------------------------------------
    algorithm_params: dict = field(default_factory=dict)
    seed: int | None = None

    # ---- quantum ----------------------------------------------------------
    backend: str | None = None           # None for classical algorithms
    qaoa_depth: int | None = None
    shots: int | None = None
    #: Per-operation error rates, or None for an ideal simulation.  Part of the
    #: record identity: the same circuit under two noise models is two runs.
    noise_model: dict | None = None
    #: Noise trajectories averaged for this point (None when not a noisy run).
    trajectories: int | None = None
    quantum_resources: dict = field(default_factory=dict)   # spec section 43
    sampling_metrics: dict = field(default_factory=dict)    # spec section 46

    # ---- solution ---------------------------------------------------------
    objective_value: float = float("nan")
    selected_inputs: list[int] = field(default_factory=list)
    covered_outcomes: list[int] = field(default_factory=list)
    n_selected: int = 0
    coverage_weight: float = 0.0
    coverage_fraction: float = 0.0
    coverage_fraction_of_achievable: float = float("nan")
    input_cost: float = 0.0
    feasible: bool = False
    certified_optimal: bool = False
    #: ``objective_value`` if the answer satisfies the constraints, else None.
    #: The native objective omits the budget penalty, so an infeasible answer
    #: can score *better* than the constrained optimum -- comparing raw
    #: objectives across records of differing feasibility produces negative
    #: gaps and a false ranking.  Only this field is comparable.
    comparable_objective: float | None = None

    # ---- quality relative to ground truth (filled by `attach_reference`) ---
    reference_objective: float | None = None
    reference_source: str | None = None
    optimality_gap_abs: float | None = None
    optimality_gap_rel: float | None = None

    # ---- the QUBO axis (filled by `attach_qubo_reference`) ----------------
    #: Energy of the delivered assignment in the QUBO actually solved.  For a
    #: native-formulation algorithm this is the energy of its selection's best
    #: completion, which places greedy on the same axis as the QUBO solvers
    #: without changing what greedy does.  None when the encoding admits no
    #: exact completion.
    qubo_energy: float | None = None
    qubo_reference_energy: float | None = None
    qubo_reference_source: str | None = None
    qubo_gap_abs: float | None = None
    qubo_gap_rel: float | None = None
    #: True when this record's `qubo_energy` is the enumerated ground state.
    qubo_certified: bool = False

    # ---- cost -------------------------------------------------------------
    runtime_ms: float = 0.0              # algorithm phase only
    timings_ms: dict = field(default_factory=dict)          # spec section 44
    wall_clock_comparable: bool = True   # False for simulated quantum

    # ---- provenance -------------------------------------------------------
    schema_version: str = SCHEMA_VERSION
    record_id: str = ""
    created_at: str = ""
    environment: dict = field(default_factory=environment_fingerprint)
    notes: str = ""
    extra: dict = field(default_factory=dict)

    def __post_init__(self):
        if self.comparable_objective is None and self.feasible:
            self.comparable_objective = self.objective_value
        if not self.created_at:
            self.created_at = datetime.now(timezone.utc).isoformat(timespec="seconds")
        if not self.record_id:
            self.record_id = self.identity_hash()

    # ------------------------------------------------------------------ ids
    def identity_hash(self) -> str:
        payload = {k: getattr(self, k) for k in IDENTITY_FIELDS}
        blob = json.dumps(payload, sort_keys=True, default=str).encode()
        return hashlib.sha256(blob).hexdigest()[:16]

    # ------------------------------------------------------------ reference
    def attach_reference(self, reference_objective: float, source: str) -> None:
        """Record the optimality gap against a *certified* optimum.

        The relative gap is normalised by ``|E*|`` (spec section 17).  When
        ``E* == 0`` the relative gap is undefined and left None rather than
        reported as zero or infinity, because either would be a false statement
        about solution quality.
        """
        self.reference_objective = float(reference_objective)
        self.reference_source = source
        if not self.feasible:
            # An infeasible answer has no optimality gap: it is not a solution
            # to this problem at all.  Recording a number here would let a
            # constraint violation read as good performance.
            self.optimality_gap_abs = None
            self.optimality_gap_rel = None
            return
        self.optimality_gap_abs = float(self.objective_value - reference_objective)
        denom = abs(float(reference_objective))
        self.optimality_gap_rel = (self.optimality_gap_abs / denom
                                   if denom > 0 else None)

    def attach_qubo_reference(self, ground_energy: float, source: str) -> None:
        """Record the gap against the QUBO's ground-state energy.

        No feasibility filter here, deliberately.  The QUBO's penalty terms
        already charge a constraint violation, so an infeasible assignment has
        a *worse* energy and its gap is a true statement about how badly the
        solver did on the function it minimised.
        """
        self.qubo_reference_energy = float(ground_energy)
        self.qubo_reference_source = source
        if self.qubo_energy is None:
            self.qubo_gap_abs = None
            self.qubo_gap_rel = None
            return
        self.qubo_gap_abs = float(self.qubo_energy - ground_energy)
        denom = abs(float(ground_energy))
        self.qubo_gap_rel = self.qubo_gap_abs / denom if denom > 0 else None

    # --------------------------------------------------------------- schema
    def as_dict(self) -> dict:
        return asdict(self)

    def to_json(self) -> str:
        return json.dumps(self.as_dict(), sort_keys=True, default=_jsonable)

    @classmethod
    def from_dict(cls, d: dict) -> "RunRecord":
        version = str(d.get("schema_version", ""))
        major = _major_of(version)
        if major != SCHEMA_MAJOR:
            raise ValueError(
                f"record schema {version!r} is not readable by this build "
                f"({SCHEMA_VERSION}): major version differs, so field meanings "
                f"may have changed. Re-run the experiment or use a matching "
                f"qbm release.")
        known = {f.name for f in fields(cls)}
        unknown = set(d) - known
        # Forward compatibility: a MINOR-newer writer may add fields.  Keep
        # them in `extra` rather than dropping them, so information is never
        # lost by a round trip through an older reader.
        kwargs = {k: v for k, v in d.items() if k in known}
        rec = cls(**kwargs)
        if unknown:
            rec.extra = dict(rec.extra)
            rec.extra["_unknown_fields"] = {k: d[k] for k in sorted(unknown)}
        return rec


def _major_of(version: str) -> int | None:
    try:
        return int(version.split("/")[1].split(".")[0])
    except Exception:
        return None


def _jsonable(o: Any):
    try:
        import numpy as np
        if isinstance(o, np.integer):
            return int(o)
        if isinstance(o, np.floating):
            return float(o)
        if isinstance(o, np.ndarray):
            return o.tolist()
        if isinstance(o, np.bool_):
            return bool(o)
    except Exception:
        pass
    return str(o)


# --------------------------------------------------------------------- sets
class ResultSet:
    """A comparison: many records sharing one instance and objective."""

    def __init__(self, records: Iterable[RunRecord] = ()):
        self.records: list[RunRecord] = list(records)

    def __len__(self):
        return len(self.records)

    def __iter__(self):
        return iter(self.records)

    def __getitem__(self, i):
        return self.records[i]

    def append(self, rec: RunRecord) -> None:
        self.records.append(rec)

    # ----------------------------------------------------------- references
    def certified_reference(self) -> tuple[float, str] | None:
        """The best certified-optimal objective in this set, if any exists."""
        certified = [r for r in self.records
                     if r.certified_optimal and r.feasible]
        if not certified:
            return None
        best = min(certified, key=lambda r: r.comparable_objective)
        return best.comparable_objective, best.algorithm

    def certified_qubo_reference(self) -> tuple[float, str] | None:
        """The enumerated QUBO ground state in this set, if one was computed."""
        certified = [r for r in self.records
                     if r.qubo_certified and r.qubo_energy is not None]
        if not certified:
            return None
        best = min(certified, key=lambda r: r.qubo_energy)
        return best.qubo_energy, best.algorithm

    def attach_references(self) -> bool:
        """Fill optimality gaps from the set's own certified reference.

        Returns False and leaves every gap None when no certified optimum is
        present -- the honest state for an instance too large to enumerate or
        solve exactly.  A gap against an uncertified best-known value would
        understate the true gap by an unknown amount.
        """
        ref = self.certified_reference()
        if ref is None:
            return False
        value, source = ref
        for r in self.records:
            r.attach_reference(value, source)
        return True

    def attach_qubo_references(self) -> bool:
        """Fill QUBO-axis gaps from an enumerated ground state, if present."""
        ref = self.certified_qubo_reference()
        if ref is None:
            return False
        value, source = ref
        for r in self.records:
            r.attach_qubo_reference(value, source)
        return True

    # ----------------------------------------------------------------- i/o
    def to_jsonl(self, path: str | Path) -> Path:
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("w") as fh:
            for r in self.records:
                fh.write(r.to_json() + "\n")
        return path

    SUMMARY_COLUMNS = (
        "benchmark_id", "instance_id", "algorithm", "formulation", "encoding",
        "qaoa_depth", "seed", "input_budget", "n_selected", "objective_value",
        "comparable_objective", "coverage_fraction", "coverage_fraction_of_achievable", "input_cost",
        "feasible", "certified_optimal", "optimality_gap_abs",
        "optimality_gap_rel", "qubo_energy", "qubo_gap_abs", "qubo_gap_rel",
        "n_problem_variables", "runtime_ms", "wall_clock_comparable",
    )

    def summary_rows(self) -> list[dict]:
        return [{k: getattr(r, k) for k in self.SUMMARY_COLUMNS}
                for r in self.records]

    def to_csv(self, path: str | Path) -> Path:
        import csv
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("w", newline="") as fh:
            wr = csv.DictWriter(fh, fieldnames=list(self.SUMMARY_COLUMNS))
            wr.writeheader()
            for row in self.summary_rows():
                wr.writerow(row)
        return path

    def to_frame(self):
        import pandas as pd
        return pd.DataFrame(self.summary_rows())


def read_jsonl(path: str | Path) -> ResultSet:
    """Read a results file, refusing records this build cannot interpret."""
    recs = []
    with Path(path).open() as fh:
        for line_no, line in enumerate(fh, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                recs.append(RunRecord.from_dict(json.loads(line)))
            except ValueError as exc:
                raise ValueError(f"{path}:{line_no}: {exc}") from None
    return ResultSet(recs)
