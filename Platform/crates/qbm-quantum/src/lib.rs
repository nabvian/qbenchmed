//! Provider-neutral contracts for optional quantum execution in Q-BenchMed.
//!
//! This crate is deliberately an integration boundary, not a provider client.
//! It performs no I/O, contains no endpoints or credential fields, and depends
//! on no quantum SDK. An application can always export the same approved,
//! content-addressed payload even when no executor is configured. Any returned
//! samples are evaluated again in the approved logical model and can be
//! rescored in the benchmark profile's native biomedical units.
//!
//! Nothing in these contracts, capability descriptions, or validation reports
//! is evidence of quantum advantage.

mod contracts;
mod equivalence;
mod error;
mod executor;
mod local;

pub use contracts::{
    ApprovalProvenance, ApprovedProblemPayload, BackendCapability, BackendKind,
    BinaryAssignmentConvention, ExecutionReadiness, ExecutionRequest, ExecutionSample,
    ExecutionUpdate, InputVariableBinding, JobFailure, JobOutcome, JobReceipt, JobResult, JobState,
    NativeRescoredSample, NativeRescoringMetadata, NativeRescoringReport, ProblemEncoding,
    QuantumProblem, ReadinessStatus,
};
pub use equivalence::{
    DEFAULT_ASSIGNMENT_LIMIT, EnergyEquivalenceConfig, EnergyEquivalenceReport,
    MAX_ASSIGNMENT_LIMIT, validate_energy_equivalence,
};
pub use error::QuantumContractError;
pub use executor::{ExecutorError, ExecutorErrorKind, QuantumExecutor};
pub use local::{LOCAL_ISING_BACKEND_ID, LocalIsingExecutor, LocalSolverConfig, coupled_variables};

/// Version of serialized backend capability reports.
pub const BACKEND_CAPABILITY_SCHEMA_VERSION: &str = "qbm.quantum-backend-capability/v1";
/// Version of serialized executor-readiness reports.
pub const EXECUTION_READINESS_SCHEMA_VERSION: &str = "qbm.quantum-readiness/v1";
/// Version of approval lineage embedded in executable payloads.
pub const APPROVAL_PROVENANCE_SCHEMA_VERSION: &str = "qbm.quantum-approval-provenance/v1";
/// Version of native-rescoring mappings.
pub const NATIVE_RESCORING_METADATA_SCHEMA_VERSION: &str =
    "qbm.quantum-native-rescoring-metadata/v1";
/// Version of immutable approved problem payloads.
pub const APPROVED_PAYLOAD_SCHEMA_VERSION: &str = "qbm.quantum-approved-payload/v1";
/// Version of immutable execution requests.
pub const EXECUTION_REQUEST_SCHEMA_VERSION: &str = "qbm.quantum-execution-request/v1";
/// Version of job receipts.
pub const JOB_RECEIPT_SCHEMA_VERSION: &str = "qbm.quantum-job-receipt/v1";
/// Version of job results.
pub const JOB_RESULT_SCHEMA_VERSION: &str = "qbm.quantum-job-result/v1";
/// Version of native-rescoring reports attached to job results.
pub const NATIVE_RESCORING_REPORT_SCHEMA_VERSION: &str = "qbm.quantum-native-rescoring-report/v1";
/// Version of executor polling updates.
pub const EXECUTION_UPDATE_SCHEMA_VERSION: &str = "qbm.quantum-execution-update/v1";
/// Version of sanitized executor errors.
pub const EXECUTOR_ERROR_SCHEMA_VERSION: &str = "qbm.quantum-executor-error/v1";
/// Version of QUBO/Ising equivalence-validation reports.
pub const ENERGY_EQUIVALENCE_SCHEMA_VERSION: &str = "qbm.quantum-energy-equivalence/v1";

/// Mandatory interpretation boundary for quantum execution output.
pub const NO_QUANTUM_ADVANTAGE_CLAIM: &str =
    "execution evidence only; no quantum advantage or biomedical superiority is claimed";
