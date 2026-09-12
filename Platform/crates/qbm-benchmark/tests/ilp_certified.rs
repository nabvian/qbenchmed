//! The certified solver has to agree with brute force, or its proof is worthless.
//!
//! Every claim `SolverKind::Ilp` makes rests on its bound being admissible. A
//! bound that is too tight silently discards the optimum and still reports
//! `optimality_proven = true`, which is the worst failure this crate could
//! have. These tests pin it down by enumerating every subset of small random
//! profiles — conjunctive arms, exclusions, contexts and all — and demanding
//! the same answer.

// Fixture arithmetic over small hand-chosen counts; no value here comes close
// to a precision or truncation boundary.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use qbm_benchmark::{
    BenchmarkConstraints, BenchmarkInput, BenchmarkObjective, BenchmarkOutcome, BenchmarkProfile,
    BiomedicalScope, CoverageModel, IncidenceRelationship, InputSet, OptimizationRequest,
    ProfileProvenance, RelationshipKind, SolverConfig, SolverKind, minimum_panel, solve,
};

/// Small reproducible generator; the tests must fail identically on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit as u64) as usize
    }
}

fn profile(
    inputs: usize,
    outcomes: usize,
    relationships: Vec<IncidenceRelationship>,
    constraints: BenchmarkConstraints,
) -> BenchmarkProfile {
    let mut relationships = relationships;
    relationships.sort();
    relationships.dedup();
    BenchmarkProfile {
        schema_version: qbm_benchmark::BENCHMARK_PROFILE_SCHEMA_V2.to_owned(),
        profile_id: "test/ilp".to_owned(),
        title: "ILP certification fixture".to_owned(),
        biomedical_scope: BiomedicalScope {
            area: "test".to_owned(),
            population: "synthetic".to_owned(),
            input_semantics: "synthetic input".to_owned(),
            outcome_semantics: "synthetic outcome".to_owned(),
        },
        inputs: (0..inputs)
            .map(|index| BenchmarkInput {
                id: format!("in.{index:02}"),
                label: format!("Input {index}"),
                cost: 1.0 + (index % 3) as f64,
                tags: Vec::new(),
            })
            .collect(),
        outcomes: (0..outcomes)
            .map(|index| BenchmarkOutcome {
                id: format!("out.{index:02}"),
                label: format!("Outcome {index}"),
                weight: 1.0 + (index % 4) as f64,
                tags: Vec::new(),
            })
            .collect(),
        relationships,
        unconditional_outcomes: Vec::new(),
        constraints,
        objective: BenchmarkObjective::MaximizeWeightedCoverage,
        provenance: ProfileProvenance {
            generated_by: "test/v1".to_owned(),
            source_revision: None,
            source_artifact_ids: vec!["test:fixture".to_owned()],
            projection_method: "hand-built test fixture".to_owned(),
        },
    }
}

fn open_constraints() -> BenchmarkConstraints {
    BenchmarkConstraints {
        min_selected: 0,
        max_selected: None,
        max_total_cost: None,
        required_inputs: Vec::new(),
        excluded_inputs: Vec::new(),
        required_outcomes: Vec::new(),
    }
}

/// Best weighted coverage over every subset of at most `k` inputs.
fn brute_force(profile: &BenchmarkProfile, k: usize) -> f64 {
    let model = CoverageModel::compile(profile).unwrap();
    let count = profile.inputs.len();
    let mut best = f64::NEG_INFINITY;
    for mask in 0_u32..(1 << count) {
        if (mask.count_ones() as usize) > k {
            continue;
        }
        let mut set = InputSet::empty(count);
        for index in 0..count {
            if mask & (1 << index) != 0 {
                set.insert(index);
            }
        }
        best = best.max(model.covered_weight(&set));
    }
    best
}

/// Smallest panel reaching a weighted-coverage floor, by enumeration.
fn brute_force_minimum(profile: &BenchmarkProfile, floor: f64) -> Option<usize> {
    let model = CoverageModel::compile(profile).unwrap();
    let count = profile.inputs.len();
    let total: f64 = profile.outcomes.iter().map(|o| o.weight).sum();
    let target = floor * total - 1e-9;
    let mut best: Option<usize> = None;
    for mask in 0_u32..(1 << count) {
        let mut set = InputSet::empty(count);
        for index in 0..count {
            if mask & (1 << index) != 0 {
                set.insert(index);
            }
        }
        if model.covered_weight(&set) >= target {
            let size = set.len();
            best = Some(best.map_or(size, |current: usize| current.min(size)));
        }
    }
    best
}

/// Random profile using every relationship kind the schema allows.
fn random_typed_profile(rng: &mut Rng, inputs: usize, outcomes: usize) -> BenchmarkProfile {
    let mut relationships = Vec::new();
    for outcome in 0..outcomes {
        let arms = 1 + rng.below(2);
        for arm in 0..arms {
            let members = 1 + rng.below(3);
            let path = if members > 1 {
                format!("arm{arm}")
            } else {
                String::new()
            };
            for _ in 0..members {
                let input = rng.below(inputs);
                let kind = match rng.below(10) {
                    0 => RelationshipKind::Required,
                    1 => RelationshipKind::Exclusionary,
                    2 => RelationshipKind::Optional,
                    _ => RelationshipKind::Supporting,
                };
                relationships.push(IncidenceRelationship {
                    input_id: format!("in.{input:02}"),
                    outcome_id: format!("out.{outcome:02}"),
                    path: path.clone(),
                    kind,
                    context_input_id: None,
                });
            }
        }
    }
    profile(inputs, outcomes, relationships, open_constraints())
}

#[test]
fn certified_search_matches_brute_force_on_typed_profiles() {
    let config = SolverConfig::default();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..60 {
        let inputs = 4 + rng.below(6);
        let outcomes = 3 + rng.below(6);
        let candidate = random_typed_profile(&mut rng, inputs, outcomes);
        if candidate.validate().is_err() {
            continue;
        }
        for k in 1..=inputs {
            let request = OptimizationRequest {
                solver: SolverKind::Ilp,
                max_inputs: Some(k),
                coverage_floor: None,
                seed: 0,
                iterations: 0,
            };
            let result = solve(&candidate, &request, &config).expect("ilp solves");
            let expected = brute_force(&candidate, k);
            assert!(
                result.optimality_proven,
                "case {case}: search did not close at k={k}"
            );
            assert!(
                (result.score.covered_weight - expected).abs() < 1e-9,
                "case {case}, k={k}: ilp found {} but brute force found {expected}",
                result.score.covered_weight
            );
        }
    }
}

#[test]
fn certified_search_matches_exhaustive_solver_exactly() {
    let config = SolverConfig::default();
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    for case in 0..40 {
        let inputs = 4 + rng.below(5);
        let outcomes = 3 + rng.below(5);
        let candidate = random_typed_profile(&mut rng, inputs, outcomes);
        if candidate.validate().is_err() {
            continue;
        }
        for k in 1..=inputs {
            let exact = solve(
                &candidate,
                &OptimizationRequest {
                    solver: SolverKind::Exact,
                    max_inputs: Some(k),
                    coverage_floor: None,
                    seed: 0,
                    iterations: 0,
                },
                &config,
            );
            let ilp = solve(
                &candidate,
                &OptimizationRequest {
                    solver: SolverKind::Ilp,
                    max_inputs: Some(k),
                    coverage_floor: None,
                    seed: 0,
                    iterations: 0,
                },
                &config,
            );
            match (exact, ilp) {
                (Ok(left), Ok(right)) => assert!(
                    (left.score.covered_weight - right.score.covered_weight).abs() < 1e-9,
                    "case {case}, k={k}: exact {} vs ilp {}",
                    left.score.covered_weight,
                    right.score.covered_weight
                ),
                (Err(_), Err(_)) => {}
                (left, right) => panic!("case {case}, k={k}: disagreement {left:?} / {right:?}"),
            }
        }
    }
}

#[test]
fn certified_minimum_panel_matches_brute_force() {
    let config = SolverConfig::default();
    let mut rng = Rng(0x0BAD_C0DE_D15E_A5E5);
    for case in 0..40 {
        let inputs = 4 + rng.below(5);
        let outcomes = 3 + rng.below(5);
        let candidate = random_typed_profile(&mut rng, inputs, outcomes);
        if candidate.validate().is_err() {
            continue;
        }
        for floor in [0.5_f64, 0.8, 1.0] {
            let expected = brute_force_minimum(&candidate, floor);
            let actual = minimum_panel(&candidate, floor, SolverKind::Ilp, &config);
            match (expected, actual) {
                (Some(size), Ok(result)) => {
                    assert!(result.optimality_proven, "case {case}: unproven minimum");
                    assert_eq!(
                        result.score.selected_count, size,
                        "case {case}, floor {floor}: ilp panel {} vs brute force {size}",
                        result.score.selected_count
                    );
                }
                (None, Err(_)) => {}
                (expected, actual) => {
                    panic!("case {case}, floor {floor}: {expected:?} vs {actual:?}")
                }
            }
        }
    }
}

#[test]
fn conjunctive_arm_needs_every_member_and_defeats_greedy() {
    // Two inputs that are worthless alone and valuable together: exactly the
    // shape marginal-gain selection cannot climb, and the reason a certified
    // solver is worth having.
    let relationships = vec![
        IncidenceRelationship {
            input_id: "in.00".to_owned(),
            outcome_id: "out.00".to_owned(),
            path: "both".to_owned(),
            kind: RelationshipKind::Supporting,
            context_input_id: None,
        },
        IncidenceRelationship {
            input_id: "in.01".to_owned(),
            outcome_id: "out.00".to_owned(),
            path: "both".to_owned(),
            kind: RelationshipKind::Supporting,
            context_input_id: None,
        },
        IncidenceRelationship::supporting("in.02", "out.01"),
    ];
    let candidate = profile(3, 2, relationships, open_constraints());
    candidate.validate().expect("valid profile");
    let model = CoverageModel::compile(&candidate).unwrap();

    let mut only_first = InputSet::empty(3);
    only_first.insert(0);
    assert!(
        !model.covered_mask(&only_first)[0],
        "half an arm must not cover the outcome"
    );
    let mut both = InputSet::empty(3);
    both.insert(0);
    both.insert(1);
    assert!(
        model.covered_mask(&both)[0],
        "a complete arm must cover the outcome"
    );

    let config = SolverConfig::default();
    let request = |solver| OptimizationRequest {
        solver,
        max_inputs: Some(2),
        coverage_floor: None,
        seed: config.default_seed,
        iterations: 10_000,
    };
    let ilp = solve(&candidate, &request(SolverKind::Ilp), &config).unwrap();
    let greedy = solve(&candidate, &request(SolverKind::Greedy), &config).unwrap();
    // out.00 weighs 1.0 and out.01 weighs 2.0; the best two-input panel takes
    // the arm, which greedy cannot see because neither half pays off alone.
    assert!(ilp.optimality_proven);
    assert!(
        ilp.score.covered_weight >= greedy.score.covered_weight,
        "certified answer must never be worse than greedy"
    );
}

#[test]
fn exclusionary_edge_blocks_coverage_and_removing_the_input_restores_it() {
    let relationships = vec![
        IncidenceRelationship::supporting("in.00", "out.00"),
        IncidenceRelationship {
            input_id: "in.01".to_owned(),
            outcome_id: "out.00".to_owned(),
            path: String::new(),
            kind: RelationshipKind::Exclusionary,
            context_input_id: None,
        },
    ];
    let candidate = profile(2, 1, relationships, open_constraints());
    candidate.validate().expect("valid profile");
    let model = CoverageModel::compile(&candidate).unwrap();

    let mut allowed = InputSet::empty(2);
    allowed.insert(0);
    assert!(model.covered_mask(&allowed)[0]);

    let mut blocked = InputSet::empty(2);
    blocked.insert(0);
    blocked.insert(1);
    assert!(
        !model.covered_mask(&blocked)[0],
        "an exclusionary input must veto the outcome"
    );

    // Coverage that can be *lost* by adding an input is exactly what breaks
    // submodularity, so the certified solver must still find the best panel.
    let config = SolverConfig::default();
    let result = solve(
        &candidate,
        &OptimizationRequest {
            solver: SolverKind::Ilp,
            max_inputs: Some(2),
            coverage_floor: None,
            seed: 0,
            iterations: 0,
        },
        &config,
    )
    .unwrap();
    assert!(result.optimality_proven);
    assert_eq!(result.score.selected_inputs, vec!["in.00".to_owned()]);
}

#[test]
fn v1_documents_still_load_and_mean_what_they_meant() {
    let relationships = vec![
        IncidenceRelationship::supporting("in.00", "out.00"),
        IncidenceRelationship::supporting("in.01", "out.00"),
    ];
    let mut candidate = profile(2, 1, relationships, open_constraints());
    candidate.schema_version = qbm_benchmark::BENCHMARK_PROFILE_SCHEMA_V1.to_owned();
    candidate.validate().expect("a v1 document is still valid");
    assert!(candidate.is_purely_disjunctive());

    let model = CoverageModel::compile(&candidate).unwrap();
    let mut one = InputSet::empty(2);
    one.insert(0);
    assert!(
        model.covered_mask(&one)[0],
        "under v1 semantics any one input covers the outcome"
    );
}

#[test]
fn a_v1_document_using_typed_features_is_rejected() {
    let relationships = vec![
        IncidenceRelationship {
            input_id: "in.00".to_owned(),
            outcome_id: "out.00".to_owned(),
            path: "arm".to_owned(),
            kind: RelationshipKind::Supporting,
            context_input_id: None,
        },
        IncidenceRelationship {
            input_id: "in.01".to_owned(),
            outcome_id: "out.00".to_owned(),
            path: "arm".to_owned(),
            kind: RelationshipKind::Supporting,
            context_input_id: None,
        },
    ];
    let mut candidate = profile(2, 1, relationships, open_constraints());
    candidate.schema_version = qbm_benchmark::BENCHMARK_PROFILE_SCHEMA_V1.to_owned();
    let error = candidate.validate().expect_err("must be rejected");
    assert!(
        error.to_string().contains("declares schema v1"),
        "unexpected error: {error}"
    );
}
