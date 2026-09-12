//! The Platform must agree with the reference implementation on the real instance.
//!
//! `QBMED-HEME-001` is the 66-input / 88-outcome rule graph exported from an
//! audit of a running pathology engine. It is the instance the Python core's
//! published results describe, and it is conjunctive: 86 of its 116 arms need
//! more than one input present at once. Until schema v2 the Platform could not
//! represent it at all, and the flattened version it *could* represent was a
//! materially easier problem.
//!
//! Every number below was produced independently by `qbm.classical.ilp`, which
//! solves the same program through a SciPy/HiGHS branch-and-bound over an LP
//! relaxation. This crate's solver shares no code and no method with it. Two
//! unrelated certified searches agreeing on a real instance is the strongest
//! evidence available that the bound here is admissible; a future change that
//! breaks it will break this file.

use qbm_benchmark::{
    BenchmarkProfile, OptimizationRequest, SolverConfig, SolverKind, minimum_panel, solve,
    structural_metrics,
};

const FIXTURE: &str = include_str!("fixtures/qbmed-heme-001.qbm.profile.json");

fn reference_profile() -> BenchmarkProfile {
    let profile: BenchmarkProfile =
        serde_json::from_str(FIXTURE).expect("the reference fixture parses");
    profile.validate().expect("the reference fixture validates");
    profile
}

fn certified_coverage(profile: &BenchmarkProfile, k: usize) -> (f64, usize) {
    let config = SolverConfig::default();
    let result = solve(
        profile,
        &OptimizationRequest {
            solver: SolverKind::Ilp,
            max_inputs: Some(k),
            coverage_floor: None,
            seed: config.default_seed,
            iterations: 0,
        },
        &config,
    )
    .expect("the certified solver returns an answer");
    assert!(
        result.optimality_proven,
        "the search did not close at k={k}; the node budget or the bound regressed"
    );
    (result.score.coverage_fraction, result.score.selected_count)
}

#[test]
fn the_reference_instance_keeps_its_conjunctive_structure() {
    let profile = reference_profile();
    assert!(
        !profile.is_purely_disjunctive(),
        "flattening this profile would delete the property that makes it hard"
    );
    let metrics = structural_metrics(&profile).unwrap();
    assert_eq!(metrics.input_count, 66);
    assert_eq!(metrics.outcome_count, 88);
    assert_eq!(metrics.relationship_count, 260);
    assert_eq!(metrics.arm_count, 116);
    assert_eq!(metrics.conjunctive_arm_count, 86);
    // Three outcomes no selection can reach, and two that fire on every
    // selection including the empty one. Both are findings about the rule set.
    assert_eq!(metrics.unreachable_outcomes.len(), 3);
    assert_eq!(metrics.unconditional_outcomes.len(), 2);
    assert_eq!(metrics.reachable_outcomes.len(), 85);
}

#[test]
fn certified_coverage_matches_the_reference_implementation() {
    let profile = reference_profile();
    // Left column: this crate. Right column: qbm.classical.ilp via HiGHS.
    for (k, expected_fraction) in [(5, 0.363_636_f64), (10, 0.568_182), (20, 0.795_455)] {
        let (fraction, selected) = certified_coverage(&profile, k);
        assert!(
            (fraction - expected_fraction).abs() < 1e-5,
            "k={k}: certified {fraction:.6} against the reference {expected_fraction:.6}"
        );
        assert_eq!(selected, k, "k={k}: the budget should be fully used here");
    }
}

#[test]
fn maximum_achievable_coverage_is_thirty_seven_inputs() {
    let profile = reference_profile();
    // 85 of 88 outcomes, reached by 37 inputs. Greedy stalls at 33 inputs and
    // 94.3% on this instance because the outcomes it cannot reach need two or
    // more inputs together, so no single input shows a positive marginal gain.
    let (fraction, selected) = certified_coverage(&profile, 37);
    assert!((fraction - 0.965_909).abs() < 1e-5, "got {fraction:.6}");
    assert_eq!(selected, 37);

    // A larger ceiling cannot buy more coverage, and must not pad the panel.
    let (wider_fraction, wider_selected) = certified_coverage(&profile, 66);
    assert!((wider_fraction - 0.965_909).abs() < 1e-5);
    assert_eq!(
        wider_selected, 37,
        "a wider ceiling returned a padded panel; redundant-input trimming regressed"
    );
}

#[test]
fn certified_minimum_panels_match_the_reference_implementation() {
    let profile = reference_profile();
    let config = SolverConfig::default();
    for (floor, expected) in [(0.8_f64, 21_usize), (0.9, 30)] {
        let result = minimum_panel(&profile, floor, SolverKind::Ilp, &config)
            .unwrap_or_else(|error| panic!("floor {floor} is unexpectedly infeasible: {error}"));
        assert!(result.optimality_proven, "floor {floor} was not proven");
        assert_eq!(
            result.score.selected_count, expected,
            "floor {floor}: certified panel {} against the reference {expected}",
            result.score.selected_count
        );
    }
}

#[test]
fn a_full_coverage_floor_is_refused_rather_than_rounded_down() {
    let profile = reference_profile();
    let config = SolverConfig::default();
    // Three outcomes are unreachable, so 100% is impossible. Saying so is the
    // whole point: silently returning the best available panel would read as a
    // success and hide a real property of the rule set.
    let error = minimum_panel(&profile, 1.0, SolverKind::Ilp, &config)
        .expect_err("a 100% floor must be refused on this instance");
    let message = error.to_string();
    assert!(
        message.contains("below the 100% floor"),
        "unexpected message: {message}"
    );
}

#[test]
fn greedy_falls_short_of_the_certified_answer_on_this_instance() {
    let profile = reference_profile();
    let config = SolverConfig::default();
    let request = |solver| OptimizationRequest {
        solver,
        max_inputs: Some(20),
        coverage_floor: None,
        seed: config.default_seed,
        iterations: 10_000,
    };
    let certified = solve(&profile, &request(SolverKind::Ilp), &config).unwrap();
    let greedy = solve(&profile, &request(SolverKind::Greedy), &config).unwrap();
    assert!(certified.optimality_proven);
    assert!(!greedy.optimality_proven);
    assert!(
        greedy.score.covered_weight <= certified.score.covered_weight + 1e-9,
        "greedy cannot beat a certified optimum"
    );
}
