//! The QUBO has to encode the problem the solvers actually solve.
//!
//! A QUBO that quietly assumes "any one selected input covers the outcome"
//! looks perfectly healthy — it builds, it converts to Ising, its energies
//! check out — while describing a strictly easier problem than the profile
//! states. Nothing downstream would catch that, because every check would be
//! internally consistent.
//!
//! So these tests do not check the encoding against itself. They enumerate the
//! whole QUBO spectrum, decode its ground state back to an input panel, and
//! demand that panel match what the certified solver proves on the native
//! model.

// Fixture arithmetic over small hand-chosen counts.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use qbm_benchmark::{
    BenchmarkConstraints, BenchmarkInput, BenchmarkObjective, BenchmarkOutcome, BenchmarkProfile,
    BiomedicalScope, IncidenceRelationship, OptimizationRequest, ProfileProvenance,
    QuboVariableKind, RelationshipKind, SolverConfig, SolverKind, build_qubo, evaluate_selection,
    solve,
};

fn profile(
    inputs: usize,
    outcomes: usize,
    relationships: Vec<IncidenceRelationship>,
    unconditional: Vec<String>,
) -> BenchmarkProfile {
    let mut relationships = relationships;
    relationships.sort();
    relationships.dedup();
    BenchmarkProfile {
        schema_version: qbm_benchmark::BENCHMARK_PROFILE_SCHEMA_V2.to_owned(),
        profile_id: "test/qubo".to_owned(),
        title: "QUBO encoding fixture".to_owned(),
        biomedical_scope: BiomedicalScope {
            area: "test".to_owned(),
            population: "synthetic".to_owned(),
            input_semantics: "synthetic input".to_owned(),
            outcome_semantics: "synthetic outcome".to_owned(),
        },
        inputs: (0..inputs)
            .map(|index| BenchmarkInput {
                id: format!("in.{index}"),
                label: format!("Input {index}"),
                cost: 1.0,
                tags: Vec::new(),
            })
            .collect(),
        outcomes: (0..outcomes)
            .map(|index| BenchmarkOutcome {
                id: format!("out.{index}"),
                label: format!("Outcome {index}"),
                weight: 1.0 + index as f64,
                tags: Vec::new(),
            })
            .collect(),
        relationships,
        unconditional_outcomes: unconditional,
        constraints: BenchmarkConstraints {
            min_selected: 0,
            max_selected: None,
            max_total_cost: None,
            required_inputs: Vec::new(),
            excluded_inputs: Vec::new(),
            required_outcomes: Vec::new(),
        },
        objective: BenchmarkObjective::MaximizeWeightedCoverage,
        provenance: ProfileProvenance {
            generated_by: "test/v1".to_owned(),
            source_revision: None,
            source_artifact_ids: vec!["test:fixture".to_owned()],
            projection_method: "hand-built test fixture".to_owned(),
        },
    }
}

fn arm(input: &str, outcome: &str, path: &str) -> IncidenceRelationship {
    IncidenceRelationship {
        input_id: input.to_owned(),
        outcome_id: outcome.to_owned(),
        path: path.to_owned(),
        kind: RelationshipKind::Supporting,
        context_input_id: None,
    }
}

/// Enumerate the whole spectrum and return the inputs selected at minimum energy.
fn ground_state_inputs(profile: &BenchmarkProfile, request: &OptimizationRequest) -> Vec<String> {
    let qubo = build_qubo(profile, request).expect("the QUBO builds");
    let count = qubo.variables.len();
    assert!(
        count <= 22,
        "fixture grew to {count} variables; enumeration would be too slow"
    );
    let mut best_energy = f64::INFINITY;
    let mut best_assignment = vec![false; count];
    for mask in 0_u32..(1 << count) {
        let assignment: Vec<bool> = (0..count).map(|bit| mask & (1 << bit) != 0).collect();
        let energy = qubo.energy(&assignment).expect("energy evaluates");
        if energy < best_energy {
            best_energy = energy;
            best_assignment = assignment;
        }
    }
    qubo.variables
        .iter()
        .filter(|variable| {
            variable.kind == QuboVariableKind::Input && best_assignment[variable.index]
        })
        .map(|variable| variable.source_id.clone())
        .collect()
}

#[test]
fn the_ground_state_agrees_with_the_certified_optimum_on_a_conjunctive_profile() {
    // out.0 (weight 1) needs in.0 alone. out.1 (weight 2) needs in.1 AND in.2
    // together. With a budget of two, taking the pair is worth more than
    // taking the single — a choice a disjunctive encoding could not represent,
    // because it would believe in.1 alone already covers out.1.
    let relationships = vec![
        arm("in.0", "out.0", ""),
        arm("in.1", "out.1", "pair"),
        arm("in.2", "out.1", "pair"),
    ];
    let candidate = profile(3, 2, relationships, Vec::new());
    candidate.validate().expect("valid profile");

    let request = OptimizationRequest {
        solver: SolverKind::Greedy,
        max_inputs: Some(2),
        coverage_floor: None,
        seed: 0,
        iterations: 0,
    };

    let decoded = ground_state_inputs(&candidate, &request);
    assert_eq!(
        decoded,
        vec!["in.1".to_owned(), "in.2".to_owned()],
        "the QUBO ground state did not take the conjunctive arm"
    );

    let certified = solve(
        &candidate,
        &OptimizationRequest {
            solver: SolverKind::Ilp,
            ..request.clone()
        },
        &SolverConfig::default(),
    )
    .expect("certified solve");
    assert!(certified.optimality_proven);
    let decoded_score = evaluate_selection(&candidate, &decoded).unwrap();
    assert!(
        (decoded_score.covered_weight - certified.score.covered_weight).abs() < 1e-9,
        "ground state covers {} but the certified optimum covers {}",
        decoded_score.covered_weight,
        certified.score.covered_weight
    );
}

#[test]
fn half_an_arm_never_lowers_the_energy() {
    // The encoding's one job: an arm variable must not be able to rise unless
    // every input it needs is selected. If it could, the QUBO would pay out
    // coverage for a rule that never fired.
    let relationships = vec![arm("in.0", "out.0", "pair"), arm("in.1", "out.0", "pair")];
    let candidate = profile(2, 1, relationships, Vec::new());
    candidate.validate().expect("valid profile");
    let request = OptimizationRequest {
        solver: SolverKind::Greedy,
        max_inputs: Some(1),
        coverage_floor: None,
        seed: 0,
        iterations: 0,
    };
    // One input is all the budget allows, and one input cannot complete the
    // arm, so the best the QUBO can do is select nothing worth selecting.
    let decoded = ground_state_inputs(&candidate, &request);
    let score = evaluate_selection(&candidate, &decoded).unwrap();
    assert!(
        score.covered_weight.abs() < 1e-9,
        "the QUBO paid out {} for an incomplete arm",
        score.covered_weight
    );
}

#[test]
fn an_arm_variable_exists_for_every_arm_and_ising_energies_match() {
    let relationships = vec![
        arm("in.0", "out.0", "a"),
        arm("in.1", "out.0", "a"),
        arm("in.2", "out.0", "b"),
        arm("in.2", "out.1", ""),
    ];
    let candidate = profile(3, 2, relationships, Vec::new());
    candidate.validate().expect("valid profile");
    let request = OptimizationRequest {
        solver: SolverKind::Greedy,
        max_inputs: Some(3),
        coverage_floor: None,
        seed: 0,
        iterations: 0,
    };
    let qubo = build_qubo(&candidate, &request).expect("builds");
    let metrics = qubo.metrics();
    // out.0 has arms "a" and "b"; out.1 has one singleton arm.
    assert_eq!(metrics.arm_variable_count, 3);

    // Every assignment must carry the same energy in both formulations, or the
    // exported Ising model is not the model that was validated.
    let ising = qubo.to_ising().expect("converts");
    let count = qubo.variables.len();
    for mask in 0_u32..(1 << count.min(16)) {
        let assignment: Vec<bool> = (0..count).map(|bit| mask & (1 << bit) != 0).collect();
        let spins: Vec<i8> = assignment
            .iter()
            .map(|value| if *value { -1 } else { 1 })
            .collect();
        let left = qubo.energy(&assignment).unwrap();
        let right = ising.energy(&spins).unwrap();
        assert!(
            (left - right).abs() < 1e-6,
            "QUBO {left} and Ising {right} disagree"
        );
    }
}

#[test]
fn an_unconditional_outcome_is_rewarded_without_an_arm() {
    let relationships = vec![arm("in.0", "out.0", "")];
    let candidate = profile(2, 2, relationships, vec!["out.1".to_owned()]);
    candidate.validate().expect("valid profile");
    let request = OptimizationRequest {
        solver: SolverKind::Greedy,
        max_inputs: Some(2),
        coverage_floor: None,
        seed: 0,
        iterations: 0,
    };
    let qubo = build_qubo(&candidate, &request).expect("builds");
    // out.1 fires for everyone, so it gets no arm and no constraint.
    assert!(
        !qubo
            .variables
            .iter()
            .any(|variable| variable.kind == QuboVariableKind::Arm
                && variable.source_id.contains("out.1")),
        "an unconditional outcome should not be gated by an arm"
    );
    let score = evaluate_selection(&candidate, &[]).unwrap();
    assert!(
        score.covered_outcomes.contains(&"out.1".to_owned()),
        "an unconditional outcome is covered by the empty selection"
    );
}
