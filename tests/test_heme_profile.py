"""Tests for the real Q-BenchMed-Heme 66x88 profile.

These tests guard the transcription of the PATHEX audit export.  They are not
clinical tests: they check that the encoded edge set agrees with what the source
document independently states about itself, and that the structural properties
downstream experiments rely on actually hold.
"""

from pathlib import Path

import numpy as np
import pytest

from qbm import profile as P
from qbm.domains import heme

PROFILE_DIR = Path(__file__).resolve().parents[1] / "benchmarks" / "heme" / "QBMED-HEME-001"


@pytest.fixture(scope="module")
def prof():
    return heme.build_profile()


def test_declared_counts_are_66_and_88(prof):
    assert len(prof.input_ids) == 66
    assert len(prof.outcome_ids) == 88
    assert len(set(prof.input_ids)) == 66
    assert len(set(prof.outcome_ids)) == 88


def test_profile_validates_against_expected_shape(prof):
    assert P.validate(prof, expect_inputs=66, expect_outcomes=88) == []


def test_encoding_reproduces_the_audit_input_split(prof):
    """The audit states 42 driving / 24 inert independently of the trigger table.

    That number was not used to build the edges, so agreement is evidence the
    transcription is right rather than a tautology.
    """
    driving = {e.input_id for e in prof.edges}
    inert = set(prof.input_ids) - driving
    assert len(driving) == heme.AUDIT_N_DRIVING_INPUTS == 42
    assert len(inert) == heme.AUDIT_N_INERT_INPUTS == 24
    assert inert == heme.AUDIT_INERT_INPUTS


def test_three_outcomes_are_unreachable_and_they_are_the_documented_ones(prof):
    cov = prof.covered(np.ones(66, dtype=int))
    unreachable = {o for o, c in zip(prof.outcome_ids, cov) if not c}
    assert unreachable == {
        "NORMAL_CBC",                                 # deprecated, no rule
        "THALASSAEMIA_TRAIT_SUSPECT",                 # deprecated, no rule
        "PROLYMPHOCYTIC_ATYPICAL_LYMPHOID_SUSPECT",   # driver outside the 66
    }
    assert int(cov.sum()) == 85


def test_only_the_two_engine_fallbacks_are_covered_by_the_empty_selection(prof):
    cov = prof.covered(np.zeros(66, dtype=int))
    assert {o for o, c in zip(prof.outcome_ids, cov) if c} == {
        "CANNOT_EVALUATE", "ANEMIA_UNCLASSIFIED"}


def test_every_edge_carries_a_source_row_and_derived_confidence(prof):
    for e in prof.edges:
        assert "row " in e.source, e
        assert "PATHEX" in e.source, e
        assert e.confidence == "derived", e
        assert e.rel_type == "supporting", e
        assert e.path, e


def test_inert_inputs_have_no_edges(prof):
    touched = {e.input_id for e in prof.edges}
    assert not (heme.AUDIT_INERT_INPUTS & touched)


def test_arm_members_are_conjunctive(prof):
    """TMA needs PLT *and* schistocytes; neither alone covers it."""
    j = prof.outcome_ids.index("TMA_SUSPECT")
    i_plt = prof.input_ids.index("PLT")
    i_sch = prof.input_ids.index("schistocytes")

    x = np.zeros(66, dtype=int)
    x[i_plt] = 1
    assert not prof.covered(x)[j]

    x = np.zeros(66, dtype=int)
    x[i_sch] = 1
    assert not prof.covered(x)[j]

    x[i_plt] = 1
    assert prof.covered(x)[j]


def test_alternative_arms_are_disjunctive(prof):
    """HIV_CONTEXT_FLAG fires on any one of HGB, WBC or PLT."""
    j = prof.outcome_ids.index("HIV_CONTEXT_FLAG")
    for name in ("HGB", "WBC", "PLT"):
        x = np.zeros(66, dtype=int)
        x[prof.input_ids.index(name)] = 1
        assert prof.covered(x)[j], name


def test_coverage_is_monotone_because_no_exclusionary_edges_exist(prof):
    """No edge removes coverage, so adding an input never reduces it.

    This is a property of *this* profile, not of the framework: the schema
    supports exclusionary edges and the audit export happens to contain none.
    """
    assert all(e.rel_type != "exclusionary" for e in prof.edges)
    rng = np.random.default_rng(7)
    for _ in range(50):
        x = rng.integers(0, 2, size=66)
        base = int(prof.covered(x).sum())
        for i in rng.choice(66, size=5, replace=False):
            y = x.copy()
            y[i] = 1
            assert int(prof.covered(y).sum()) >= base


def test_coverage_is_not_submodular(prof):
    """Conjunctive arms create increasing marginal returns.

    Greedy's 1-1/e guarantee for maximum coverage assumes submodularity.  A
    two-input arm breaks it: the second input of the pair is worth nothing until
    the first is present, so the marginal gain of an input can *increase* as the
    selection grows.  This is why the real profile can discriminate between
    greedy and exact optimization instead of reporting a tie, and it is recorded
    as a test so a later schema change cannot quietly remove the property.
    """
    j = prof.outcome_ids.index("MARROW_INFILTRATION_SUSPECT")
    i_tear = prof.input_ids.index("teardrop_dacrocytes")
    i_nrbc = prof.input_ids.index("nucleated_rbc")

    empty = np.zeros(66, dtype=int)
    only_tear = empty.copy()
    only_tear[i_tear] = 1

    def gain(x, i):
        y = x.copy()
        y[i] = 1
        return int(prof.covered(y)[j]) - int(prof.covered(x)[j])

    # Marginal value of nucleated_rbc rises from 0 to 1 once teardrops are in.
    assert gain(empty, i_nrbc) == 0
    assert gain(only_tear, i_nrbc) == 1


def test_costs_and_weights_are_uniform_and_flagged_unjustified(prof):
    assert set(prof.input_costs.values()) == {1.0}
    assert set(prof.outcome_weights.values()) == {1.0}
    assert prof.provenance["cost_model_justified"] is False
    assert prof.provenance["real_export_present"] is True
    assert prof.provenance["synthetic_stand_in"] is False


def test_dropped_arms_are_recorded_with_reasons(prof):
    dropped = prof.provenance["dropped_arms"]
    assert {d["outcome"] for d in dropped} == {
        "CLL_CLPD_SUSPECT", "PROLYMPHOCYTIC_ATYPICAL_LYMPHOID_SUSPECT"}
    for d in dropped:
        assert "registry" in d["reason"]


def test_saved_profile_on_disk_matches_the_builder(prof):
    """The committed benchmark directory is the artifact experiments read."""
    loaded = P.DomainProfile.load(PROFILE_DIR)
    assert loaded.checksum() == prof.checksum()
    rng = np.random.default_rng(11)
    for _ in range(25):
        x = rng.integers(0, 2, size=66)
        assert (loaded.covered(x) == prof.covered(x)).all()


def test_structural_findings_report_the_four_interpretation_caveats(prof):
    text = " ".join(P.structural_findings(prof))
    assert "unreachable" in text
    assert "effective decision dimension is 42" in text
    assert "constant" in text
    assert "alternative rule arms" in text
