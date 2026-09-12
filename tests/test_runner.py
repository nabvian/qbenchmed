"""The result schema and the common runner.

The runner exists so that spec section 50's fairness requirement is enforced by
construction rather than by discipline: one code path builds the problem and
hands the identical object to every algorithm.  These tests check the two places
where that guarantee could still be lost -- an algorithm receiving a different
problem, and a record being scored or compared on a different basis.
"""

import json

import numpy as np
import pytest

from qbm import results as rs
from qbm import runner as rn
from qbm.instances import Instance, generate


@pytest.fixture
def inst():
    return generate("clustered", 8, 10, seed=17)


@pytest.fixture
def spec():
    return rn.RunSpec(benchmark_id="TEST-001", input_budget=3,
                      benchmark_version="1.0.0")


# --------------------------------------------------------------- schema rules
def test_record_id_is_determined_by_the_configuration(inst, spec):
    a = rn.run_one(inst, "greedy", spec)
    b = rn.run_one(inst, "greedy", spec)
    assert a.record_id == b.record_id           # same configuration, same id
    c = rn.run_one(inst, "greedy",
                   rn.RunSpec(benchmark_id="TEST-001", input_budget=4))
    assert c.record_id != a.record_id           # different budget, different id


def test_reader_refuses_a_record_from_a_different_major_version(tmp_path):
    rec = rs.RunRecord(benchmark_id="b", instance_id="i", instance_checksum="c",
                       algorithm="greedy", algorithm_version="v")
    d = rec.as_dict()
    d["schema_version"] = "qbm-result/2.0.0"
    p = tmp_path / "r.jsonl"
    p.write_text(json.dumps(d) + "\n")
    with pytest.raises(ValueError, match="not readable by this build"):
        rs.read_jsonl(p)


def test_unknown_fields_from_a_newer_minor_writer_are_kept(tmp_path):
    rec = rs.RunRecord(benchmark_id="b", instance_id="i", instance_checksum="c",
                       algorithm="greedy", algorithm_version="v")
    d = rec.as_dict()
    d["schema_version"] = f"qbm-result/{rs.SCHEMA_MAJOR}.9.0"
    d["a_future_metric"] = 1.25
    back = rs.RunRecord.from_dict(d)
    assert back.extra["_unknown_fields"]["a_future_metric"] == 1.25


def test_environment_fingerprint_carries_no_host_identity():
    env = rs.environment_fingerprint()
    assert {"python", "platform", "machine", "cpu_count", "packages"} <= set(env)
    blob = json.dumps(env).lower()
    for leak in ("hostname", "/users/", "/home/", "user"):
        assert leak not in blob


# ------------------------------------------------------- feasibility and gaps
def _record(**kw):
    base = dict(benchmark_id="b", instance_id="i", instance_checksum="c",
                algorithm="x", algorithm_version="v")
    base.update(kw)
    return rs.RunRecord(**base)


def test_infeasible_answer_has_no_comparable_objective():
    r = _record(objective_value=-9.0, feasible=False)
    assert r.comparable_objective is None
    r2 = _record(objective_value=-9.0, feasible=True)
    assert r2.comparable_objective == -9.0


def test_infeasible_answer_gets_no_optimality_gap():
    """A constraint violation must never read as better-than-optimal."""
    r = _record(objective_value=-9.0, feasible=False)
    r.attach_reference(-7.0, "exhaustive")
    assert r.optimality_gap_abs is None
    assert r.optimality_gap_rel is None
    assert r.reference_objective == -7.0        # the reference is still recorded


def test_relative_gap_is_undefined_when_the_optimum_is_zero():
    r = _record(objective_value=0.5, feasible=True)
    r.attach_reference(0.0, "exhaustive")
    assert r.optimality_gap_abs == pytest.approx(0.5)
    assert r.optimality_gap_rel is None


def test_certified_reference_ignores_an_infeasible_certificate():
    good = _record(algorithm="ilp", objective_value=-7.0, feasible=True,
                   certified_optimal=True)
    bogus = _record(algorithm="broken", objective_value=-99.0, feasible=False,
                    certified_optimal=True)
    ref = rs.ResultSet([bogus, good]).certified_reference()
    assert ref == (-7.0, "ilp")


def test_no_certified_optimum_means_no_gaps_at_all():
    s = rs.ResultSet([_record(objective_value=-5.0, feasible=True),
                      _record(objective_value=-6.0, feasible=True)])
    assert s.attach_references() is False
    assert all(r.optimality_gap_abs is None for r in s)


# ----------------------------------------------------------- fairness by path
def test_exact_solvers_agree_and_certify(inst, spec):
    e = rn.run_one(inst, "exhaustive", spec)
    i = rn.run_one(inst, "ilp", spec)
    assert e.certified_optimal and i.certified_optimal
    assert e.objective_value == pytest.approx(i.objective_value)


def test_every_algorithm_sees_the_same_instance_and_budget(inst, spec):
    out = rn.run_comparison(inst, spec, [
        {"algorithm": "exhaustive"}, {"algorithm": "greedy"},
        {"algorithm": "ilp"}, {"algorithm": "simulated_annealing", "seed": 3},
    ])
    checksums = {r.instance_checksum for r in out}
    budgets = {r.input_budget for r in out}
    objectives = {r.objective for r in out}
    assert len(checksums) == 1 and len(budgets) == 1 and len(objectives) == 1
    assert all(r.n_selected <= spec.input_budget for r in out if r.feasible)


def test_no_solver_beats_the_certified_optimum(inst, spec):
    out = rn.run_comparison(inst, spec, [
        {"algorithm": "exhaustive"}, {"algorithm": "greedy"},
        {"algorithm": "simulated_annealing", "seed": 1},
        {"algorithm": "tabu", "seed": 1},
    ])
    ref, _ = out.certified_reference()
    for r in out:
        if r.feasible:
            assert r.comparable_objective >= ref - 1e-9
            assert r.optimality_gap_abs >= -1e-9


def test_exact_algorithms_run_before_the_rest(inst, spec):
    order = []
    rn.run_comparison(inst, spec,
                      [{"algorithm": "greedy"}, {"algorithm": "ilp"}],
                      progress=order.append)
    assert order[0].startswith("ilp")


def test_unknown_algorithm_is_rejected(inst, spec):
    with pytest.raises(KeyError, match="unknown algorithm"):
        rn.run_one(inst, "quantum_annealer", spec)


# ------------------------------------------------------------------ reporting
def test_coverage_is_reported_against_both_denominators(spec):
    """An unreachable outcome caps coverage for reasons unrelated to the solver."""
    arms = [(0, (0,)), (1, (1,))]                # outcome 2 has no arm
    A = np.zeros((2, 3), dtype=np.uint8)
    A[0, 0] = A[1, 1] = 1
    inst = Instance(instance_id="capped", regime="test", seed=0, A=A,
                    w=np.ones(3), c=np.ones(2), arms=arms)
    assert rn.achievable_coverage_weight(inst) == pytest.approx(2.0)
    rec = rn.run_one(inst, "exhaustive", spec)
    assert rec.coverage_fraction == pytest.approx(2 / 3)
    assert rec.coverage_fraction_of_achievable == pytest.approx(1.0)
    assert rec.n_reachable_outcomes == 2


def test_classical_runtime_is_marked_comparable_and_decomposed(inst, spec):
    r = rn.run_one(inst, "greedy", spec)
    assert r.wall_clock_comparable is True
    assert {"total", "algorithm"} <= set(r.timings_ms)
    assert r.timings_ms["total"] >= r.timings_ms["algorithm"]


def test_results_round_trip_through_jsonl_and_csv(tmp_path, inst, spec):
    out = rn.run_comparison(inst, spec, [{"algorithm": "exhaustive"},
                                         {"algorithm": "greedy"}])
    p = out.to_jsonl(tmp_path / "results.jsonl")
    csv_path = out.to_csv(tmp_path / "summary.csv")
    back = rs.read_jsonl(p)
    assert [r.record_id for r in back] == [r.record_id for r in out]
    assert [r.objective_value for r in back] == [r.objective_value for r in out]
    header = csv_path.read_text().splitlines()[0].split(",")
    assert "comparable_objective" in header and "wall_clock_comparable" in header


# ------------------------------------------------------------------ QAOA path
@pytest.mark.slow
def test_qaoa_runs_through_the_same_runner_and_is_not_wall_clock_comparable():
    A = np.zeros((3, 3), dtype=np.uint8)
    arms = [(0, (0,)), (1, (1,)), (2, (2,))]
    for j, mem in arms:
        A[mem[0], j] = 1
    inst = Instance(instance_id="q", regime="test", seed=0, A=A,
                    w=np.ones(3), c=np.ones(3), arms=arms)
    spec = rn.RunSpec(benchmark_id="Q-TEST", input_budget=1)
    out = rn.run_comparison(inst, spec, [
        {"algorithm": "exhaustive"},
        {"algorithm": "qaoa", "depth": 1, "shots": 256, "seed": 0, "maxiter": 25},
    ])
    q = [r for r in out if r.algorithm == "qaoa"][0]
    assert q.wall_clock_comparable is False      # simulated, not QPU time
    assert q.backend == "statevector"
    assert q.qaoa_depth == 1 and q.shots == 256
    assert q.quantum_resources["resources"]["n_qubits"] == q.n_problem_variables
    assert q.sampling_metrics["measured_against_certified_optimum"] is True
    assert {"parameter_optimization", "qubo_build"} <= set(q.timings_ms)
    if q.feasible:
        assert q.n_selected <= 1
        assert q.comparable_objective >= out.certified_reference()[0] - 1e-9
