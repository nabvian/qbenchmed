//! Integration tests for immutable quantum execution contracts.

use std::str::FromStr;

use chrono::{TimeZone, Utc};
use qbm_benchmark::{
    BENCHMARK_PROFILE_SCHEMA_VERSION, BenchmarkConstraints, BenchmarkInput, BenchmarkObjective,
    BenchmarkOutcome, BenchmarkProfile, BiomedicalScope, IncidenceRelationship,
    OptimizationRequest, ProfileProvenance, QUBO_SCHEMA_VERSION, QuboCoupling, QuboModel,
    QuboVariable, QuboVariableKind, SolverKind, build_qubo,
};
use qbm_canonical::{hash_value, sha256_bytes};
use qbm_domain::{ActorId, Approval, ApprovalId, Decision, RunId, StageId};
use qbm_quantum::{
    APPROVED_PAYLOAD_SCHEMA_VERSION, BACKEND_CAPABILITY_SCHEMA_VERSION, BackendCapability,
    BackendKind, EnergyEquivalenceConfig, ExecutionReadiness, ExecutionRequest, ExecutionSample,
    ExecutionUpdate, ExecutorError, JobFailure, JobOutcome, JobReceipt, JobResult, JobState,
    NO_QUANTUM_ADVANTAGE_CLAIM, ProblemEncoding, QuantumContractError, QuantumExecutor,
    ReadinessStatus, validate_energy_equivalence,
};

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 29, 12, 34, 56)
        .single()
        .unwrap()
}

fn profile() -> BenchmarkProfile {
    BenchmarkProfile {
        schema_version: BENCHMARK_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id: "panel-v1".to_owned(),
        title: "Reviewed assay panel".to_owned(),
        biomedical_scope: BiomedicalScope {
            area: "oncology".to_owned(),
            population: "reviewed-cohort".to_owned(),
            input_semantics: "assay".to_owned(),
            outcome_semantics: "covered-phenotype".to_owned(),
        },
        inputs: vec![
            BenchmarkInput {
                id: "assay-a".to_owned(),
                label: "Assay A".to_owned(),
                cost: 1.0,
                tags: vec!["rna".to_owned()],
            },
            BenchmarkInput {
                id: "assay-b".to_owned(),
                label: "Assay B".to_owned(),
                cost: 2.0,
                tags: vec!["serum".to_owned()],
            },
        ],
        outcomes: vec![
            BenchmarkOutcome {
                id: "phenotype-a".to_owned(),
                label: "Phenotype A".to_owned(),
                weight: 3.0,
                tags: vec!["primary".to_owned()],
            },
            BenchmarkOutcome {
                id: "phenotype-b".to_owned(),
                label: "Phenotype B".to_owned(),
                weight: 2.0,
                tags: vec!["secondary".to_owned()],
            },
        ],
        relationships: vec![
            IncidenceRelationship {
                input_id: "assay-a".to_owned(),
                outcome_id: "phenotype-a".to_owned(),
            },
            IncidenceRelationship {
                input_id: "assay-b".to_owned(),
                outcome_id: "phenotype-a".to_owned(),
            },
            IncidenceRelationship {
                input_id: "assay-b".to_owned(),
                outcome_id: "phenotype-b".to_owned(),
            },
        ],
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
            generated_by: "test-fixture/v1".to_owned(),
            source_revision: Some("0123456789abcdef".to_owned()),
            source_artifact_ids: vec!["artifact-1".to_owned()],
            projection_method: "manual reviewed fixture".to_owned(),
        },
    }
}

fn qubo() -> QuboModel {
    build_qubo(&profile(), &OptimizationRequest::new(SolverKind::Exact)).unwrap()
}

fn approval_for<T: serde::Serialize>(value: &T, decision: Decision) -> Approval {
    Approval {
        id: ApprovalId::from_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap(),
        run_id: RunId::from_str("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap(),
        stage_id: StageId::new("quantum-payload").unwrap(),
        stage_output_hash: hash_value(value).unwrap(),
        decision,
        actor_id: ActorId::new("reviewer-1").unwrap(),
        reason: None,
        created_at: fixed_time(),
    }
}

fn payload() -> qbm_quantum::ApprovedProblemPayload {
    let model = qubo();
    qbm_quantum::ApprovedProblemPayload::from_qubo(
        model.clone(),
        &profile(),
        approval_for(&model, Decision::Approve),
    )
    .unwrap()
}

fn capability() -> BackendCapability {
    BackendCapability {
        schema_version: BACKEND_CAPABILITY_SCHEMA_VERSION.to_owned(),
        backend_id: "fixture-simulator".to_owned(),
        provider: "test-provider".to_owned(),
        display_name: "Fixture simulator".to_owned(),
        backend_kind: BackendKind::Simulator,
        supported_encodings: vec![ProblemEncoding::Qubo, ProblemEncoding::Ising],
        maximum_variables: Some(128),
        maximum_couplings: Some(1_024),
        maximum_samples_per_request: Some(10_000),
        supports_seed: true,
        supports_polling: true,
        limitations: vec!["test fixture only".to_owned()],
    }
}

#[test]
fn small_converted_model_is_checked_exhaustively() {
    let qubo = qubo();
    let ising = qubo.to_ising().unwrap();
    let report = validate_energy_equivalence(
        &qubo,
        &ising,
        EnergyEquivalenceConfig {
            assignment_limit: 4_096,
            ..EnergyEquivalenceConfig::default()
        },
    )
    .unwrap();

    assert!(report.exhaustive);
    assert_eq!(
        report.assignments_checked as u64,
        report.assignment_space_size.unwrap()
    );
    assert!(report.maximum_absolute_error <= 1.0e-10);
    assert!(report.limitation.contains("does not evaluate hardware"));
}

fn large_qubo(variable_count: usize) -> QuboModel {
    QuboModel {
        schema_version: QUBO_SCHEMA_VERSION.to_owned(),
        profile_id: "large-profile".to_owned(),
        request: OptimizationRequest::new(SolverKind::Exact),
        variables: (0..variable_count)
            .map(|index| QuboVariable {
                index,
                name: format!("x-{index:03}"),
                kind: QuboVariableKind::Slack,
                source_id: format!("source-{index:03}"),
            })
            .collect(),
        linear: (0..variable_count)
            .map(|index| f64::from(u32::try_from(index).unwrap()) / 17.0 - 2.0)
            .collect(),
        quadratic: vec![QuboCoupling {
            left: 0,
            right: variable_count - 1,
            coefficient: -0.75,
        }],
        offset: 1.25,
        constraint_penalty: 1.0,
        encoding_notes: Vec::new(),
    }
}

#[test]
fn large_validation_is_bounded_and_deterministic() {
    let qubo = large_qubo(70);
    let ising = qubo.to_ising().unwrap();
    let config = EnergyEquivalenceConfig {
        assignment_limit: 37,
        ..EnergyEquivalenceConfig::default()
    };
    let first = validate_energy_equivalence(&qubo, &ising, config).unwrap();
    let second = validate_energy_equivalence(&qubo, &ising, config).unwrap();

    assert_eq!(first, second);
    assert!(!first.exhaustive);
    assert_eq!(first.assignment_space_size, None);
    assert_eq!(first.assignments_checked, 37);
    assert!(first.limitation.contains("not an exhaustive proof"));
}

#[test]
fn energy_mismatch_reports_first_generated_assignment() {
    let qubo = large_qubo(70);
    let mut ising = qubo.to_ising().unwrap();
    ising.offset += 0.5;

    let error =
        validate_energy_equivalence(&qubo, &ising, EnergyEquivalenceConfig::default()).unwrap_err();
    assert!(matches!(
        error,
        QuantumContractError::EnergyMismatch {
            assignment_index: 0,
            ..
        }
    ));
}

#[test]
fn equivalence_config_rejects_unbounded_or_nonfinite_work() {
    let qubo = qubo();
    let ising = qubo.to_ising().unwrap();
    let zero = validate_energy_equivalence(
        &qubo,
        &ising,
        EnergyEquivalenceConfig {
            assignment_limit: 0,
            ..EnergyEquivalenceConfig::default()
        },
    );
    assert!(zero.is_err());

    let nonfinite = validate_energy_equivalence(
        &qubo,
        &ising,
        EnergyEquivalenceConfig {
            absolute_tolerance: f64::NAN,
            ..EnergyEquivalenceConfig::default()
        },
    );
    assert!(nonfinite.is_err());
}

#[test]
fn approved_payload_round_trips_and_detects_tampering() {
    let payload = payload();
    payload.validate().unwrap();
    assert_eq!(payload.schema_version, APPROVED_PAYLOAD_SCHEMA_VERSION);
    assert_eq!(
        payload.provenance.approval.stage_output_hash,
        payload.provenance.source_model_sha256
    );

    let encoded = serde_json::to_vec(&payload).unwrap();
    let decoded: qbm_quantum::ApprovedProblemPayload = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, payload);
    decoded.validate().unwrap();

    let mut tampered = decoded;
    tampered.native_rescoring.profile_sha256 = sha256_bytes(b"different profile").unwrap();
    assert!(matches!(
        tampered.validate().unwrap_err(),
        QuantumContractError::IdentityMismatch { .. }
    ));
}

#[test]
fn approved_ising_payload_keeps_qubo_native_mapping() {
    let qubo = qubo();
    let ising = qubo.to_ising().unwrap();
    let (payload, report) = qbm_quantum::ApprovedProblemPayload::from_equivalent_ising(
        &qubo,
        ising.clone(),
        &profile(),
        approval_for(&ising, Decision::Approve),
        EnergyEquivalenceConfig::default(),
    )
    .unwrap();

    payload.validate().unwrap();
    assert!(report.exhaustive);
    assert_eq!(payload.problem.encoding(), ProblemEncoding::Ising);
    assert_eq!(payload.native_rescoring.input_variables.len(), 2);
    assert_eq!(
        payload.native_rescoring.source_qubo_sha256,
        hash_value(&qubo).unwrap()
    );
}

#[test]
fn payload_requires_approve_decision_on_exact_model_digest() {
    let model = qubo();
    let rejected = qbm_quantum::ApprovedProblemPayload::from_qubo(
        model.clone(),
        &profile(),
        approval_for(&model, Decision::Reject),
    );
    assert!(matches!(
        rejected.unwrap_err(),
        QuantumContractError::InvalidContract(_)
    ));

    let mut wrong_approval = approval_for(&model, Decision::Approve);
    wrong_approval.stage_output_hash = sha256_bytes(b"some other stage output").unwrap();
    let wrong = qbm_quantum::ApprovedProblemPayload::from_qubo(model, &profile(), wrong_approval);
    assert!(matches!(
        wrong.unwrap_err(),
        QuantumContractError::InvalidContract(_)
    ));
}

#[test]
fn serde_contracts_reject_unknown_fields() {
    let mut value = serde_json::to_value(payload()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("credential".to_owned(), serde_json::json!("must-not-exist"));
    assert!(serde_json::from_value::<qbm_quantum::ApprovedProblemPayload>(value).is_err());
}

#[test]
fn readiness_distinguishes_export_only_and_not_configured() {
    let export_only = ExecutionReadiness::export_only();
    let not_configured = ExecutionReadiness::not_configured();
    export_only.validate().unwrap();
    not_configured.validate().unwrap();

    assert_eq!(export_only.status, ReadinessStatus::ExportOnly);
    assert_eq!(not_configured.status, ReadinessStatus::NotConfigured);
    assert_eq!(
        serde_json::to_value(&export_only).unwrap()["status"],
        "export_only"
    );
    assert_eq!(
        serde_json::to_value(&not_configured).unwrap()["status"],
        "not_configured"
    );
    assert!(
        serde_json::to_string(&export_only)
            .unwrap()
            .contains(NO_QUANTUM_ADVANTAGE_CLAIM)
    );
}

#[test]
fn request_preflight_enforces_backend_capabilities_and_identity() {
    let request = ExecutionRequest::new(payload(), "fixture-simulator", 200)
        .unwrap()
        .with_seed(42)
        .unwrap();
    request.validate_for_backend(&capability()).unwrap();

    let mut too_small = capability();
    too_small.maximum_variables = Some(1);
    assert!(request.validate_for_backend(&too_small).is_err());

    let mut no_seed = capability();
    no_seed.supports_seed = false;
    assert!(request.validate_for_backend(&no_seed).is_err());

    let mut tampered = request;
    tampered.sample_count += 1;
    assert!(matches!(
        tampered.validate().unwrap_err(),
        QuantumContractError::IdentityMismatch { .. }
    ));
}

fn selected_a_assignment(payload: &qbm_quantum::ApprovedProblemPayload) -> Vec<bool> {
    let mut assignment = vec![false; payload.problem.variable_count()];
    let binding = payload
        .native_rescoring
        .input_variables
        .iter()
        .find(|binding| binding.input_id == "assay-a")
        .unwrap();
    assignment[binding.variable_index] = true;
    assignment
}

#[test]
fn receipt_result_and_native_rescore_preserve_full_lineage() {
    let request = ExecutionRequest::new(payload(), "fixture-simulator", 50).unwrap();
    let receipt = JobReceipt::new(&request, "job-123", JobState::Succeeded, fixed_time()).unwrap();
    let sample = ExecutionSample::new(
        &request.payload.problem,
        selected_a_assignment(&request.payload),
        None,
        7,
    )
    .unwrap();
    let result = JobResult::succeeded(&request, &receipt, vec![sample], fixed_time())
        .unwrap()
        .with_native_rescoring(&request, &receipt, &profile())
        .unwrap();

    result.validate_against(&request, &receipt).unwrap();
    result
        .validate_native_rescoring(&request, &profile())
        .unwrap();
    assert_eq!(result.outcome, JobOutcome::Succeeded);
    assert_eq!(result.request_sha256, request.request_sha256);
    assert_eq!(result.payload_sha256, request.payload.payload_sha256);
    let score = &result.native_rescoring.as_ref().unwrap().samples[0].score;
    assert_eq!(score.selected_inputs, vec!["assay-a"]);
    assert_eq!(score.covered_outcomes, vec!["phenotype-a"]);
    assert!(
        result
            .limitations
            .contains(&NO_QUANTUM_ADVANTAGE_CLAIM.to_owned())
    );

    let update = ExecutionUpdate::complete(&request, receipt, result).unwrap();
    update.validate(&request).unwrap();
}

#[test]
fn logical_energy_and_result_identity_are_tamper_evident() {
    let request = ExecutionRequest::new(payload(), "fixture-simulator", 5).unwrap();
    let receipt =
        JobReceipt::new(&request, "job-energy", JobState::Succeeded, fixed_time()).unwrap();
    let sample = ExecutionSample::new(
        &request.payload.problem,
        selected_a_assignment(&request.payload),
        Some(-123.0),
        1,
    )
    .unwrap();
    let mut result = JobResult::succeeded(&request, &receipt, vec![sample], fixed_time()).unwrap();
    result.samples[0].logical_energy += 1.0;

    assert!(result.validate_against(&request, &receipt).is_err());
}

#[test]
fn failed_and_cancelled_results_have_strict_terminal_shapes() {
    let request = ExecutionRequest::new(payload(), "fixture-simulator", 5).unwrap();
    let failed_receipt =
        JobReceipt::new(&request, "job-failed", JobState::Failed, fixed_time()).unwrap();
    let failed = JobResult::failed(
        &request,
        &failed_receipt,
        JobFailure {
            code: "capacity".to_owned(),
            summary: "backend capacity unavailable".to_owned(),
            retryable: true,
        },
        fixed_time(),
    )
    .unwrap();
    failed.validate_against(&request, &failed_receipt).unwrap();

    let cancelled_receipt =
        JobReceipt::new(&request, "job-cancelled", JobState::Cancelled, fixed_time()).unwrap();
    let cancelled = JobResult::cancelled(&request, &cancelled_receipt, fixed_time()).unwrap();
    cancelled
        .validate_against(&request, &cancelled_receipt)
        .unwrap();
}

struct MockExecutor {
    capability: BackendCapability,
}

impl QuantumExecutor for MockExecutor {
    fn capability(&self) -> &BackendCapability {
        &self.capability
    }

    fn submit(&self, request: &ExecutionRequest) -> Result<JobReceipt, ExecutorError> {
        request
            .validate_for_backend(&self.capability)
            .map_err(|error| {
                ExecutorError::new(
                    qbm_quantum::ExecutorErrorKind::Unsupported,
                    "invalid_request",
                    error.to_string(),
                    false,
                )
            })?;
        JobReceipt::new(request, "mock-job", JobState::Submitted, fixed_time()).map_err(|error| {
            ExecutorError::new(
                qbm_quantum::ExecutorErrorKind::InvalidResponse,
                "invalid_receipt",
                error.to_string(),
                false,
            )
        })
    }

    fn poll(
        &self,
        request: &ExecutionRequest,
        receipt: &JobReceipt,
    ) -> Result<ExecutionUpdate, ExecutorError> {
        ExecutionUpdate::pending(request, receipt.clone()).map_err(|error| {
            ExecutorError::new(
                qbm_quantum::ExecutorErrorKind::InvalidResponse,
                "invalid_update",
                error.to_string(),
                false,
            )
        })
    }
}

#[test]
fn executor_trait_is_object_safe_and_provider_neutral() {
    let executor: Box<dyn QuantumExecutor> = Box::new(MockExecutor {
        capability: capability(),
    });
    let request = ExecutionRequest::new(payload(), "fixture-simulator", 5).unwrap();
    assert_eq!(executor.readiness().status, ReadinessStatus::Ready);
    let receipt = executor.submit(&request).unwrap();
    assert_eq!(receipt.state, JobState::Submitted);
    assert!(executor.poll(&request, &receipt).unwrap().result.is_none());

    let serialized = serde_json::to_string(&request).unwrap();
    for forbidden in ["credential", "api_key", "authorization", "endpoint"] {
        assert!(!serialized.contains(forbidden));
    }
}
