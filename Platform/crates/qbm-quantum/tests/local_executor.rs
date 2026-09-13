//! The local executor has to solve the model the Platform actually exported.
//!
//! Its value is not that it returns low energies — any minimiser does that. It
//! is that decoding its answer back to an input panel must land on the same
//! panel the certified native solver proves. If those disagree, the exported
//! formulation describes a different problem from the profile, and nothing
//! else in the pipeline would have noticed.
//!
//! So the tests below check the conversion against the contract's own energy
//! function, the minimum against brute force, and the decoded panel against
//! `SolverKind::Ilp`.

#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use std::str::FromStr;

use chrono::{TimeZone, Utc};
use qbm_benchmark::{
    BENCHMARK_PROFILE_SCHEMA_V2, BenchmarkConstraints, BenchmarkInput, BenchmarkObjective,
    BenchmarkOutcome, BenchmarkProfile, BiomedicalScope, IncidenceRelationship,
    OptimizationRequest, ProfileProvenance, RelationshipKind, SolverConfig, SolverKind, build_qubo,
    evaluate_selection, solve,
};
use qbm_canonical::hash_value;
use qbm_domain::{ActorId, Approval, ApprovalId, Decision, RunId, StageId};
use qbm_quantum::{
    ApprovedProblemPayload, ExecutionRequest, JobOutcome, JobState, LOCAL_ISING_BACKEND_ID,
    LocalIsingExecutor, LocalSolverConfig, QuantumExecutor, QuantumProblem, ReadinessStatus,
};

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 13, 9, 0, 0).single().unwrap()
}

fn approval_for<T: serde::Serialize>(value: &T) -> Approval {
    Approval {
        id: ApprovalId::from_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap(),
        run_id: RunId::from_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap(),
        stage_id: StageId::new("quantum-payload").unwrap(),
        stage_output_hash: hash_value(value).unwrap(),
        decision: Decision::Approve,
        actor_id: ActorId::new("reviewer-1").unwrap(),
        reason: None,
        created_at: fixed_time(),
    }
}

/// A profile whose best two-input panel is a conjunctive arm.
///
/// `out.pair` (weight 3) needs both `in.a` and `in.b`; `out.single` (weight 1)
/// needs `in.c` alone. A disjunctive reading would believe `in.a` alone already
/// covers `out.pair`, and would pick a different panel.
fn conjunctive_profile() -> BenchmarkProfile {
    let mut relationships = vec![
        IncidenceRelationship {
            input_id: "in.a".to_owned(),
            outcome_id: "out.pair".to_owned(),
            path: "both".to_owned(),
            kind: RelationshipKind::Supporting,
            context_input_id: None,
        },
        IncidenceRelationship {
            input_id: "in.b".to_owned(),
            outcome_id: "out.pair".to_owned(),
            path: "both".to_owned(),
            kind: RelationshipKind::Supporting,
            context_input_id: None,
        },
        IncidenceRelationship::supporting("in.c", "out.single"),
    ];
    relationships.sort();
    BenchmarkProfile {
        schema_version: BENCHMARK_PROFILE_SCHEMA_V2.to_owned(),
        profile_id: "test/local-executor".to_owned(),
        title: "Local executor fixture".to_owned(),
        biomedical_scope: BiomedicalScope {
            area: "test".to_owned(),
            population: "synthetic".to_owned(),
            input_semantics: "synthetic input".to_owned(),
            outcome_semantics: "synthetic outcome".to_owned(),
        },
        inputs: ["in.a", "in.b", "in.c"]
            .into_iter()
            .map(|id| BenchmarkInput {
                id: id.to_owned(),
                label: id.to_owned(),
                cost: 1.0,
                tags: Vec::new(),
            })
            .collect(),
        outcomes: [("out.pair", 3.0), ("out.single", 1.0)]
            .into_iter()
            .map(|(id, weight)| BenchmarkOutcome {
                id: id.to_owned(),
                label: id.to_owned(),
                weight,
                tags: Vec::new(),
            })
            .collect(),
        relationships,
        unconditional_outcomes: Vec::new(),
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

fn request_for(profile: &BenchmarkProfile, max_inputs: usize, samples: u32) -> ExecutionRequest {
    let model = build_qubo(
        profile,
        &OptimizationRequest {
            solver: SolverKind::Greedy,
            max_inputs: Some(max_inputs),
            coverage_floor: None,
            seed: 0,
            iterations: 0,
        },
    )
    .expect("the QUBO builds");
    let payload = ApprovedProblemPayload::from_qubo(model.clone(), profile, approval_for(&model))
        .expect("payload");
    ExecutionRequest::new(payload, LOCAL_ISING_BACKEND_ID, samples)
        .expect("request")
        .with_seed(7)
        .expect("seeded request")
}

#[test]
fn the_backend_declares_itself_classical_and_never_claims_otherwise() {
    let executor = LocalIsingExecutor::default();
    let capability = executor.capability();
    assert_eq!(capability.backend_id, LOCAL_ISING_BACKEND_ID);
    assert_eq!(capability.backend_kind, qbm_quantum::BackendKind::Simulator);
    assert_eq!(executor.readiness().status, ReadinessStatus::Ready);

    let text = capability.limitations.join(" ").to_lowercase();
    assert!(
        text.contains("not a quantum processor"),
        "the backend must say plainly that it is not quantum hardware"
    );
    assert!(
        text.contains("quantum advantage"),
        "the no-advantage boundary has to travel with the capability"
    );

    // A capability is shown to users and serialized into reports; it must not
    // look like a configured provider connection.
    let serialized = serde_json::to_string(capability).unwrap();
    for forbidden in ["endpoint", "api_key", "credential", "token", "http"] {
        assert!(
            !serialized.to_lowercase().contains(forbidden),
            "capability leaked {forbidden}"
        );
    }
}

#[test]
fn the_bit_space_conversion_agrees_with_the_contract_on_both_encodings() {
    // The solver works in its own bit-space form for speed. If that conversion
    // is wrong, every energy it reports is wrong in a way nothing else checks.
    let profile = conjunctive_profile();
    let request = request_for(&profile, 3, 4);
    let qubo_problem = &request.payload.problem;
    let QuantumProblem::Qubo(model) = qubo_problem else {
        panic!("expected a QUBO payload");
    };
    let ising_problem = QuantumProblem::Ising(model.to_ising().unwrap());

    let executor = LocalIsingExecutor::default();
    let count = qubo_problem.variable_count();
    // Exhaustive minimisation reports the energy it computed itself; compare
    // that with the contract's independent evaluation of the same assignment.
    for problem in [qubo_problem, &ising_problem] {
        let found = executor.minimize(problem, 8, 0).expect("minimises");
        assert!(!found.is_empty());
        for (assignment, energy, _) in &found {
            assert_eq!(assignment.len(), count);
            let expected = problem.energy(assignment).expect("contract energy");
            assert!(
                (expected - energy).abs() < 1e-9,
                "solver energy {energy} against contract energy {expected}"
            );
        }
    }
}

#[test]
fn exhaustive_minimisation_finds_the_true_minimum() {
    let profile = conjunctive_profile();
    let request = request_for(&profile, 3, 1);
    let problem = &request.payload.problem;
    let count = problem.variable_count();
    assert!(count <= 22, "fixture must stay inside the exhaustive bound");

    let executor = LocalIsingExecutor::default();
    let found = executor.minimize(problem, 1, 0).expect("minimises");
    let (_, reported, _) = &found[0];

    let mut brute = f64::INFINITY;
    for mask in 0_u64..(1 << count) {
        let assignment: Vec<bool> = (0..count).map(|bit| mask & (1 << bit) != 0).collect();
        brute = brute.min(problem.energy(&assignment).unwrap());
    }
    assert!(
        (brute - reported).abs() < 1e-9,
        "solver reported {reported} against brute-force {brute}"
    );
}

#[test]
fn the_solved_model_decodes_to_the_certified_panel() {
    // The reason this executor exists. If the exported formulation encodes the
    // profile's problem, its ground state decodes to the panel the certified
    // native solver proves optimal.
    let profile = conjunctive_profile();
    let request = request_for(&profile, 2, 4);
    let executor = LocalIsingExecutor::default();

    let receipt = executor.submit(&request).expect("submitted");
    assert_eq!(receipt.state, JobState::Running);
    let update = executor.poll(&request, &receipt).expect("polled");
    let result = update.result.expect("a terminal poll carries a result");
    assert_eq!(result.outcome, JobOutcome::Succeeded);
    assert!(!result.samples.is_empty());

    // Rescoring is the host's job, not the executor's: the executor never sees
    // a benchmark profile, which is what keeps it provider-neutral. The host
    // holds the profile and attaches the native scores here.
    assert!(
        result.native_rescoring.is_none(),
        "the executor must not carry biomedical semantics"
    );
    let rescored = result
        .with_native_rescoring(&request, &update.receipt, &profile)
        .expect("the host rescores samples in native units");
    let rescoring = rescored.native_rescoring.as_ref().unwrap();
    let best = &rescoring.samples[0];
    let certified = solve(
        &profile,
        &OptimizationRequest {
            solver: SolverKind::Ilp,
            max_inputs: Some(2),
            coverage_floor: None,
            seed: SolverConfig::default().default_seed,
            iterations: 0,
        },
        &SolverConfig::default(),
    )
    .expect("certified solve");
    assert!(certified.optimality_proven);

    assert_eq!(
        best.score.selected_inputs, certified.score.selected_inputs,
        "the ground state decoded to a different panel from the certified optimum"
    );
    assert!(
        (best.score.covered_weight - certified.score.covered_weight).abs() < 1e-9,
        "decoded coverage {} against certified {}",
        best.score.covered_weight,
        certified.score.covered_weight
    );
    // And the panel it picked is the conjunctive arm, not the cheap single.
    let arm = evaluate_selection(&profile, &["in.a".to_owned(), "in.b".to_owned()]).unwrap();
    assert!((best.score.covered_weight - arm.covered_weight).abs() < 1e-9);
}

#[test]
fn polling_twice_returns_the_same_answer() {
    // The executor holds no state, so a second poll recomputes. That is only
    // acceptable if it recomputes identically.
    let profile = conjunctive_profile();
    let request = request_for(&profile, 3, 4);
    let executor = LocalIsingExecutor::default();
    let receipt = executor.submit(&request).expect("submitted");

    let first = executor.poll(&request, &receipt).expect("first poll");
    let terminal = first.receipt.clone();
    let second = executor.poll(&request, &terminal).expect("second poll");

    assert_eq!(terminal.state, JobState::Succeeded);
    assert_eq!(
        first.result.as_ref().map(|r| &r.samples),
        second.result.as_ref().map(|r| &r.samples),
        "two polls of one job disagreed"
    );
    assert_eq!(
        second.receipt.receipt_sha256, terminal.receipt_sha256,
        "polling a terminal receipt must not change its identity"
    );
}

#[test]
fn a_model_larger_than_the_declared_ceiling_is_refused_not_attempted() {
    let executor = LocalIsingExecutor::new(LocalSolverConfig {
        maximum_variables: 4,
        ..LocalSolverConfig::default()
    });
    let profile = conjunctive_profile();
    let request = request_for(&profile, 3, 1);
    assert!(
        request.payload.problem.variable_count() > 4,
        "fixture must exceed the ceiling for this test to mean anything"
    );
    let error = executor
        .submit(&request)
        .expect_err("a model above the declared limit must be refused");
    assert_eq!(error.kind, qbm_quantum::ExecutorErrorKind::Unsupported);
}

#[test]
fn the_heuristic_path_is_seeded_and_reproducible() {
    // Force the heuristic by setting the exhaustive bound below the model.
    let executor = LocalIsingExecutor::new(LocalSolverConfig {
        exhaustive_max_variables: 2,
        restarts: 16,
        iterations_per_restart: 500,
        ..LocalSolverConfig::default()
    });
    let profile = conjunctive_profile();
    let request = request_for(&profile, 3, 3);
    let problem = &request.payload.problem;

    let first = executor.minimize(problem, 3, 42).expect("minimises");
    let second = executor.minimize(problem, 3, 42).expect("minimises");
    assert_eq!(first, second, "the same seed gave different samples");

    // It should still reach the true minimum on a fixture this small, and must
    // never report an energy below it.
    let count = problem.variable_count();
    let mut brute = f64::INFINITY;
    for mask in 0_u64..(1 << count) {
        let assignment: Vec<bool> = (0..count).map(|bit| mask & (1 << bit) != 0).collect();
        brute = brute.min(problem.energy(&assignment).unwrap());
    }
    assert!(
        first[0].1 >= brute - 1e-9,
        "the heuristic reported an energy below the true minimum"
    );
    assert!(
        (first[0].1 - brute).abs() < 1e-9,
        "the heuristic missed the minimum on a fixture this small: {} against {brute}",
        first[0].1
    );
}
