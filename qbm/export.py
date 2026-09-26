"""Export a `DomainProfile` as the Rust Platform's `qbm.benchmark-profile`.

The Platform is the part of Q-BenchMed you point at a project; `qbm.profile`
is the document it reads.  Until schema v2 that document could only express
binary incidence -- a flat list of (input, outcome) pairs -- so exporting a
real rule set meant flattening away its conjunctions.  That flattening was not
a cosmetic loss: conjunctivity is precisely what makes weighted coverage
non-submodular, what strands outcomes no selection can reach, and what stops
greedy at 33 inputs on QBMED-HEME-001 while the certified optimum continues to
37.  An export that dropped it would have handed the Platform an instance that
looked easy because the hard part had been deleted.

Schema v2 carries relationship kinds and path groups, so the projection is now
lossless for every profile this module can express, and `export_profile`
refuses rather than quietly degrading when it meets something it cannot carry.

    from qbm.domains import heme
    from qbm.export import write_benchmark_profile

    write_benchmark_profile(heme.build(), "out/qbm.profile.json")

The Platform validates the result strictly -- sorted, unique, no dangling
references -- so everything this module emits is ordered on the way out rather
than left to the reader to fix.
"""

from __future__ import annotations

import json
from pathlib import Path

from .profile import CONTRIBUTING, REL_TYPES, DomainProfile

SCHEMA_VERSION = "qbm.benchmark-profile/v2"

# The Platform's identifier grammar: ASCII letters, digits and these symbols.
# `#` is admitted in a path label only, where it is the conventional separator
# between an outcome and one of its rule arms.
_ID_CHARS = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._:/-")
_PATH_CHARS = _ID_CHARS | {"#"}


class ExportError(ValueError):
    """Raised when a profile cannot be expressed in the target schema.

    Refusing is the point.  A silent downgrade would produce a document the
    Platform accepts and scores, whose numbers describe a different problem
    from the one the profile states.
    """


def _check_id(value: str, field: str, allowed: set[str] = _ID_CHARS) -> str:
    if not value or len(value) > 256:
        raise ExportError(f"{field} must contain 1..=256 bytes, got {value!r}")
    bad = sorted(set(value) - allowed)
    if bad:
        raise ExportError(
            f"{field} {value!r} contains characters the Platform rejects: {bad}. "
            f"Rename it in the source profile rather than rewriting it here -- "
            f"an identifier silently changed on export no longer matches the "
            f"audit rows it came from."
        )
    return value


def _label(identifier: str) -> str:
    """A display label for an identifier that carries none of its own."""
    return identifier.replace("_", " ").replace("#", " / ")


def build_benchmark_profile(
    profile: DomainProfile,
    *,
    profile_id: str | None = None,
    title: str | None = None,
    population: str = "Not declared by the source profile",
    input_semantics: str = "A selectable biomarker the rule engine can read",
    outcome_semantics: str = "A declared decision-support target the rules can fire",
    constraints: dict | None = None,
) -> dict:
    """Project `profile` onto the `qbm.benchmark-profile/v2` document shape.

    Every edge keeps its type, its path group and its context input, so the
    Platform evaluates coverage by the same rule this package does.  Nothing is
    inferred: identifiers, costs, weights and arm membership are carried across
    unchanged, and a value the source does not state is defaulted visibly here
    rather than guessed.
    """
    for identifier in profile.input_ids:
        _check_id(identifier, "input id")
    for identifier in profile.outcome_ids:
        _check_id(identifier, "outcome id")

    known_inputs = set(profile.input_ids)
    known_outcomes = set(profile.outcome_ids)

    inputs = [
        {
            "id": identifier,
            "label": _label(identifier),
            "cost": float(profile.input_costs.get(identifier, 1.0)),
            "tags": [],
        }
        for identifier in sorted(known_inputs)
    ]
    outcomes = [
        {
            "id": identifier,
            "label": _label(identifier),
            "weight": float(profile.outcome_weights.get(identifier, 1.0)),
            "tags": [],
        }
        for identifier in sorted(known_outcomes)
    ]
    for entry in inputs:
        if not entry["cost"] > 0:
            raise ExportError(f"input {entry['id']!r} has non-positive cost {entry['cost']}")
    for entry in outcomes:
        if not entry["weight"] > 0:
            raise ExportError(
                f"outcome {entry['id']!r} has non-positive weight {entry['weight']}"
            )

    relationships = []
    for edge in profile.edges:
        if edge.rel_type not in REL_TYPES:
            raise ExportError(f"unknown relationship type {edge.rel_type!r}")
        if edge.input_id not in known_inputs:
            raise ExportError(f"edge references unknown input {edge.input_id!r}")
        if edge.outcome_id not in known_outcomes:
            raise ExportError(f"edge references unknown outcome {edge.outcome_id!r}")
        if edge.path:
            _check_id(edge.path, "relationship path", _PATH_CHARS)

        row: dict = {"input_id": edge.input_id, "outcome_id": edge.outcome_id}
        if edge.path:
            row["path"] = edge.path
        if edge.rel_type != "supporting":
            row["kind"] = edge.rel_type
        if edge.rel_type == "contextual":
            if not edge.context_input_id:
                raise ExportError(
                    f"contextual edge {edge.input_id}->{edge.outcome_id} declares no "
                    f"context input; the Platform cannot evaluate it"
                )
            if edge.context_input_id not in known_inputs:
                raise ExportError(
                    f"edge context references unknown input {edge.context_input_id!r}"
                )
            row["context_input_id"] = edge.context_input_id
        elif edge.context_input_id:
            raise ExportError(
                f"edge {edge.input_id}->{edge.outcome_id} is {edge.rel_type} but names a "
                f"context input; only contextual edges may"
            )
        relationships.append(row)

    # The schema orders relationships by (input, outcome, path, kind, context)
    # and rejects duplicates. Two identical rows carry no extra meaning, so
    # collapsing them here is lossless -- unlike dropping a path would be.
    def sort_key(row: dict) -> tuple:
        return (
            row["input_id"],
            row["outcome_id"],
            row.get("path", ""),
            REL_TYPES.index(row.get("kind", "supporting")),
            row.get("context_input_id") or "",
        )

    relationships.sort(key=sort_key)
    deduplicated: list[dict] = []
    for row in relationships:
        if not deduplicated or sort_key(deduplicated[-1]) != sort_key(row):
            deduplicated.append(row)

    if not any(
        row.get("kind", "supporting") in CONTRIBUTING for row in deduplicated
    ):
        raise ExportError(
            "no relationship can grant coverage; the exported profile would score "
            "zero for every selection"
        )

    unconditional = sorted(set(profile.unconditional_outcomes))
    for identifier in unconditional:
        if identifier not in known_outcomes:
            raise ExportError(f"unconditional outcome {identifier!r} is not declared")

    declared = dict(constraints or {})
    document = {
        "schema_version": SCHEMA_VERSION,
        "profile_id": _check_id(
            profile_id or f"biomedical/{profile.benchmark_id.lower()}", "profile_id"
        ),
        "title": title or f"{profile.benchmark_id} ({profile.domain})",
        "biomedical_scope": {
            "area": profile.domain,
            "population": population,
            "input_semantics": input_semantics,
            "outcome_semantics": outcome_semantics,
        },
        "inputs": inputs,
        "outcomes": outcomes,
        "relationships": deduplicated,
        "constraints": {
            "min_selected": int(declared.get("min_selected", 0)),
            "max_selected": declared.get("max_selected"),
            "max_total_cost": declared.get("max_total_cost"),
            "required_inputs": sorted(set(declared.get("required_inputs", []))),
            "excluded_inputs": sorted(set(declared.get("excluded_inputs", []))),
            "required_outcomes": sorted(set(declared.get("required_outcomes", []))),
        },
        "objective": "maximize_weighted_coverage",
        "provenance": {
            "generated_by": "qbm.export/v2",
            "source_revision": str(profile.provenance.get("source_revision") or "")
            or None,
            "source_artifact_ids": sorted(
                {_check_id(f"benchmark:{profile.benchmark_id}", "source_artifact_id")}
            ),
            "projection_method": (
                profile.notes
                or "direct projection of the domain profile onto qbm.benchmark-profile/v2"
            ),
        },
    }
    if unconditional:
        document["unconditional_outcomes"] = unconditional
    return document


def write_benchmark_profile(profile: DomainProfile, path: str | Path, **kwargs) -> Path:
    """Write the projected document, creating parent directories as needed."""
    document = build_benchmark_profile(profile, **kwargs)
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    return target
