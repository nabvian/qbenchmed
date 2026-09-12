"""Tests for the domain-profile adapter boundary (spec sections 21, 26)."""

from __future__ import annotations

import itertools

import numpy as np
import pytest

from qbm import instances as ins
from qbm.profile import (CONFIDENCE_CLASSES, REL_TYPES, DomainProfile,
                         ProfileError, Relationship, structural_findings,
                         validate)


def _profile_from_instance(inst: ins.Instance, benchmark_id="T") -> DomainProfile:
    """Build a binary-equivalent profile from a core Instance."""
    edges = []
    for i, j in zip(*np.nonzero(inst.A)):
        edges.append(Relationship(
            input_id=inst.input_names[i], outcome_id=inst.outcome_names[j],
            rel_type="supporting", source="test_fixture"))
    return DomainProfile(
        benchmark_id=benchmark_id, domain="test",
        input_ids=list(inst.input_names), outcome_ids=list(inst.outcome_names),
        edges=edges,
        outcome_weights={o: float(w) for o, w in zip(inst.outcome_names, inst.w)},
        input_costs={c: float(v) for c, v in zip(inst.input_names, inst.c)},
    )


# --------------------------------------------------------------------------
# backward compatibility -- the load-bearing test
# --------------------------------------------------------------------------
@pytest.mark.parametrize("regime", ["low_overlap", "high_overlap", "degree_capped"])
def test_binary_profile_coverage_matches_instance_on_every_subset(regime):
    """A supporting-only profile must reproduce Instance coverage exactly.

    If this fails, introducing the typed model silently changed the meaning of
    every previously published result.
    """
    inst = ins.generate(regime, 8, 10, seed=3)
    prof = _profile_from_instance(inst)
    assert prof.is_binary_equivalent()
    for bits in itertools.product([False, True], repeat=inst.n_inputs):
        x = np.array(bits)
        np.testing.assert_array_equal(prof.covered(x), inst.covered(x))
        assert prof.coverage_weight(x) == pytest.approx(inst.coverage_weight(x))


def test_to_instance_roundtrip_is_lossless_for_binary_profile():
    inst = ins.generate("clustered", 10, 12, seed=1)
    prof = _profile_from_instance(inst, benchmark_id=inst.instance_id)
    back = prof.to_instance(instance_id=inst.instance_id)
    np.testing.assert_array_equal(back.A, inst.A)
    np.testing.assert_allclose(back.w, inst.w)
    np.testing.assert_allclose(back.c, inst.c)
    assert "LOSSY" not in back.notes


def test_to_instance_flags_lossy_projection_for_typed_profile():
    prof = DomainProfile(
        benchmark_id="T", domain="test",
        input_ids=["a", "b"], outcome_ids=["o1"],
        edges=[Relationship("a", "o1", "supporting", source="s"),
               Relationship("b", "o1", "exclusionary", source="s")],
    )
    assert not prof.is_binary_equivalent()
    assert "LOSSY" in prof.to_instance().notes


# --------------------------------------------------------------------------
# typed semantics
# --------------------------------------------------------------------------
def test_required_edge_blocks_coverage_until_present():
    prof = DomainProfile(
        benchmark_id="T", domain="test",
        input_ids=["req", "sup"], outcome_ids=["o1"],
        edges=[Relationship("req", "o1", "required", source="s"),
               Relationship("sup", "o1", "supporting", source="s")],
    )
    assert not prof.covered([False, True])[0]   # supporting alone: not covered
    assert prof.covered([True, False])[0]       # required alone: covered
    assert prof.covered([True, True])[0]


def test_exclusionary_edge_removes_coverage():
    prof = DomainProfile(
        benchmark_id="T", domain="test",
        input_ids=["sup", "bad"], outcome_ids=["o1"],
        edges=[Relationship("sup", "o1", "supporting", source="s"),
               Relationship("bad", "o1", "exclusionary", source="s")],
    )
    assert prof.covered([True, False])[0]
    assert not prof.covered([True, True])[0]    # adding an input UNCOVERS


def test_exclusionary_makes_objective_non_submodular():
    """Non-submodularity is the point: greedy's 1-1/e guarantee needs submodular f.

    Submodularity requires marginal gain to be non-increasing as the set grows.
    An exclusionary edge produces a strictly *negative* marginal gain for an
    input that had positive gain earlier, which violates it.
    """
    prof = DomainProfile(
        benchmark_id="T", domain="test",
        input_ids=["a", "b"], outcome_ids=["o1", "o2"],
        edges=[Relationship("a", "o1", "supporting", source="s"),
               Relationship("b", "o2", "supporting", source="s"),
               Relationship("b", "o1", "exclusionary", source="s")],
    )
    f = prof.coverage_weight
    empty = np.array([False, False])
    gain_b_alone = f(np.array([False, True])) - f(empty)
    gain_b_after_a = f(np.array([True, True])) - f(np.array([True, False]))
    assert gain_b_alone == pytest.approx(1.0)
    assert gain_b_after_a == pytest.approx(0.0)   # b now costs o1 to gain o2
    assert gain_b_after_a < gain_b_alone


def test_contextual_edge_fires_only_with_context():
    prof = DomainProfile(
        benchmark_id="T", domain="test",
        input_ids=["ctx", "dep"], outcome_ids=["o1"],
        edges=[Relationship("dep", "o1", "contextual", context_input_id="ctx",
                            source="s")],
    )
    assert not prof.covered([False, True])[0]   # dep without ctx: no fire
    assert prof.covered([True, True])[0]


def test_optional_edge_carries_reduced_credit():
    prof = DomainProfile(
        benchmark_id="T", domain="test",
        input_ids=["a"], outcome_ids=["o1"],
        edges=[Relationship("a", "o1", "optional", source="s")],
    )
    assert prof.relationships()[0].credit() == pytest.approx(0.5)


# --------------------------------------------------------------------------
# serialization and provenance
# --------------------------------------------------------------------------
def test_save_load_roundtrip(tmp_path):
    inst = ins.generate("sparse", 6, 8, seed=5)
    prof = _profile_from_instance(inst, benchmark_id="RT-1")
    prof.provenance = {"knowledge_version": "1.0", "real_export_present": False}
    d = prof.save(tmp_path / "RT-1")
    back = DomainProfile.load(d)
    assert back.checksum() == prof.checksum()
    assert back.input_ids == prof.input_ids
    assert back.outcome_ids == prof.outcome_ids
    assert len(back.edges) == len(prof.edges)
    for x in ([True] * inst.n_inputs, [False] * inst.n_inputs):
        np.testing.assert_array_equal(back.covered(np.array(x)),
                                      prof.covered(np.array(x)))


def test_checksum_detects_edited_profile(tmp_path):
    prof = DomainProfile(
        benchmark_id="T", domain="test", input_ids=["a"], outcome_ids=["o1"],
        edges=[Relationship("a", "o1", "supporting", source="s")])
    d = prof.save(tmp_path / "T")
    rc = d / "relationships.csv"
    rc.write_text(rc.read_text().replace("supporting", "optional"))
    with pytest.raises(ProfileError, match="checksum mismatch"):
        DomainProfile.load(d)
    # ...and loads fine when the caller explicitly opts out
    assert DomainProfile.load(d, verify_checksum=False).edges[0].rel_type == "optional"


def test_missing_files_raise_actionable_error(tmp_path):
    (tmp_path / "empty").mkdir()
    with pytest.raises(ProfileError, match="missing profile.yaml"):
        DomainProfile.load(tmp_path / "empty")


def test_ragged_relationship_row_names_the_line(tmp_path):
    prof = DomainProfile(
        benchmark_id="T", domain="test", input_ids=["a"], outcome_ids=["o1"],
        edges=[Relationship("a", "o1", "supporting", source="s")])
    d = prof.save(tmp_path / "T")
    rc = d / "relationships.csv"
    rc.write_text(rc.read_text() + "a,o1\n")
    with pytest.raises(ProfileError, match=r"relationships\.csv:3 has 2 fields"):
        DomainProfile.load(d, verify_checksum=False)


# --------------------------------------------------------------------------
# validator -- every error path
# --------------------------------------------------------------------------
def _valid() -> DomainProfile:
    return DomainProfile(
        benchmark_id="V", domain="test",
        input_ids=["a", "b"], outcome_ids=["o1", "o2"],
        edges=[Relationship("a", "o1", "supporting", source="s"),
               Relationship("b", "o2", "supporting", source="s")],
        outcome_weights={"o1": 1.0, "o2": 2.0},
        input_costs={"a": 1.0, "b": 1.0},
    )


def test_valid_profile_has_no_errors():
    assert validate(_valid()) == []


def test_validator_collects_all_errors_not_just_first():
    p = _valid()
    p.edges = [Relationship("ghost", "o1", "supporting", source=""),
               Relationship("a", "nowhere", "bogus_type", source="")]
    errs = validate(p)
    assert len(errs) >= 4, errs
    joined = " ".join(errs)
    for expect in ("unknown input id", "missing source", "unknown outcome id",
                   "not one of"):
        assert expect in joined, (expect, errs)


@pytest.mark.parametrize("mutate,expect", [
    (lambda p: p.input_ids.append("a"), "duplicate input ids"),
    (lambda p: p.outcome_ids.append("o1"), "duplicate outcome ids"),
    (lambda p: p.edges.append(Relationship("a", "o1", "supporting", source="s")),
     "duplicate edge"),
    (lambda p: p.edges.append(Relationship("a", "o1", "contextual", source="s")),
     "needs a context_input_id"),
    (lambda p: p.edges.append(Relationship("a", "o2", "contextual",
                                           context_input_id="a", source="s")),
     "its own context"),
    (lambda p: p.edges.append(Relationship("a", "o2", "supporting", weight=-1.0,
                                           source="s")), "must be finite and >= 0"),
    (lambda p: p.edges.append(Relationship("a", "o2", "supporting", source="s",
                                           confidence="vibes")), "not in"),
    (lambda p: p.outcome_weights.__setitem__("o1", 0.0), "must be finite and > 0"),
    (lambda p: p.input_costs.__setitem__("a", -3.0), "must be finite and > 0"),
])
def test_validator_error_paths(mutate, expect):
    p = _valid()
    mutate(p)
    errs = validate(p)
    assert any(expect in e for e in errs), (expect, errs)


def test_unreachable_outcome_is_a_finding_not_an_error():
    """A real export can contain a deprecated outcome; it must still load."""
    p = DomainProfile(
        benchmark_id="V", domain="test", input_ids=["a"], outcome_ids=["o1", "orphan"],
        edges=[Relationship("a", "o1", "supporting", source="s")])
    assert validate(p) == []
    f = structural_findings(p)
    assert any("unreachable" in s and "orphan" in s for s in f), f


def test_inert_input_is_a_finding_reporting_effective_dimension():
    p = DomainProfile(
        benchmark_id="V", domain="test", input_ids=["a", "deadweight"],
        outcome_ids=["o1"],
        edges=[Relationship("a", "o1", "supporting", source="s")])
    assert validate(p) == []
    f = structural_findings(p)
    assert any("deadweight" in s and "effective decision dimension is 1" in s
               for s in f), f


# --------------------------------------------------------------------------
# path groups
# --------------------------------------------------------------------------
def test_path_group_requires_all_members():
    p = DomainProfile(
        benchmark_id="P", domain="test", input_ids=["a", "b", "c"],
        outcome_ids=["o"],
        edges=[Relationship("a", "o", "supporting", source="s", path="A"),
               Relationship("b", "o", "supporting", source="s", path="A")])
    assert validate(p) == []
    cov = lambda *sel: bool(p.covered(
        np.array([i in sel for i in p.input_ids]))[0])
    assert not cov("a")
    assert not cov("b")
    assert cov("a", "b")
    assert cov("a", "b", "c")
    assert not p.is_binary_equivalent()


def test_alternative_paths_are_disjunctive():
    p = DomainProfile(
        benchmark_id="P", domain="test", input_ids=["a", "b", "c"],
        outcome_ids=["o"],
        edges=[Relationship("a", "o", "supporting", source="s", path="A"),
               Relationship("b", "o", "supporting", source="s", path="A"),
               Relationship("c", "o", "supporting", source="s", path="B")])
    cov = lambda *sel: bool(p.covered(
        np.array([i in sel for i in p.input_ids]))[0])
    assert cov("c")            # arm B alone
    assert cov("a", "b")       # arm A alone
    assert not cov("a")        # arm A incomplete, arm B unsatisfied


def test_unconditional_outcome_is_covered_by_empty_selection():
    p = DomainProfile(
        benchmark_id="P", domain="test", input_ids=["a"], outcome_ids=["o", "fallback"],
        edges=[Relationship("a", "o", "supporting", source="s")],
        unconditional_outcomes=["fallback"])
    assert validate(p) == []
    cov = p.covered(np.zeros(1, dtype=bool))
    assert cov[1] and not cov[0]
    assert p.covered(np.ones(1, dtype=bool)).all()
    assert not p.is_binary_equivalent()
    assert any("empty selection" in s for s in structural_findings(p))


def test_unconditional_outcome_must_exist():
    p = DomainProfile(
        benchmark_id="P", domain="test", input_ids=["a"], outcome_ids=["o"],
        edges=[Relationship("a", "o", "supporting", source="s")],
        unconditional_outcomes=["ghost"])
    assert any("not a declared outcome" in e for e in validate(p))


def test_path_survives_round_trip(tmp_path):
    p = DomainProfile(
        benchmark_id="P", domain="test", input_ids=["a", "b"], outcome_ids=["o"],
        edges=[Relationship("a", "o", "supporting", source="s", path="A"),
               Relationship("b", "o", "supporting", source="s", path="A")],
        unconditional_outcomes=[])
    p.save(tmp_path / "prof")
    q = DomainProfile.load(tmp_path / "prof")
    assert [e.path for e in q.edges] == ["A", "A"]
    assert q.checksum() == p.checksum()


def test_validator_enforces_expected_shape():
    errs = validate(_valid(), expect_inputs=66, expect_outcomes=88)
    assert any("expected 66 inputs" in e for e in errs)
    assert any("expected 88 outcomes" in e for e in errs)


def test_benchmark_metadata_reports_edge_types_and_flags():
    p = _valid()
    p.provenance = {"real_export_present": False, "synthetic_stand_in": True,
                    "knowledge_version": "0.9"}
    m = p.benchmark_metadata()
    assert m["n_inputs"] == 2 and m["n_outcomes"] == 2 and m["n_edges"] == 2
    assert m["edges_by_type"]["supporting"] == 2
    assert m["edges_by_type"]["exclusionary"] == 0
    assert m["is_binary_equivalent"] is True
    assert m["synthetic_stand_in"] is True
    assert m["real_export_present"] is False
    assert m["knowledge_version"] == "0.9"
    assert set(REL_TYPES) == set(m["edges_by_type"])
    assert "defined" in CONFIDENCE_CLASSES
