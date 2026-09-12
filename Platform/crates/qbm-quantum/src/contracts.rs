//! Versioned execution and provenance contracts.

use std::{cmp::Ordering, collections::BTreeSet};

use chrono::{DateTime, Utc};
use qbm_benchmark::{
    BenchmarkProfile, IsingModel, QUBO_SCHEMA_VERSION, QuboModel, QuboVariableKind, SelectionScore,
    evaluate_selection,
};
use qbm_canonical::hash_value;
use qbm_domain::{Approval, Decision, Sha256Digest};
use serde::{Deserialize, Serialize};

use crate::{
    APPROVAL_PROVENANCE_SCHEMA_VERSION, APPROVED_PAYLOAD_SCHEMA_VERSION,
    BACKEND_CAPABILITY_SCHEMA_VERSION, EXECUTION_READINESS_SCHEMA_VERSION,
    EXECUTION_REQUEST_SCHEMA_VERSION, EXECUTION_UPDATE_SCHEMA_VERSION, JOB_RECEIPT_SCHEMA_VERSION,
    JOB_RESULT_SCHEMA_VERSION, NATIVE_RESCORING_METADATA_SCHEMA_VERSION,
    NATIVE_RESCORING_REPORT_SCHEMA_VERSION, NO_QUANTUM_ADVANTAGE_CLAIM, QuantumContractError,
    error::require_schema,
};

/// Serialized problem encoding accepted by an executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemEncoding {
    /// Binary quadratic unconstrained optimization.
    Qubo,
    /// Pairwise Ising Hamiltonian with spins in `{-1, +1}`.
    Ising,
}

/// Approved logical problem carried by a portable execution payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "model", rename_all = "snake_case")]
pub enum QuantumProblem {
    /// QUBO represented by the benchmark crate's reviewed schema.
    Qubo(QuboModel),
    /// Ising model represented by the benchmark crate's reviewed schema.
    Ising(IsingModel),
}

impl QuantumProblem {
    /// Return the problem encoding.
    #[must_use]
    pub const fn encoding(&self) -> ProblemEncoding {
        match self {
            Self::Qubo(_) => ProblemEncoding::Qubo,
            Self::Ising(_) => ProblemEncoding::Ising,
        }
    }

    /// Return the stable benchmark profile identity.
    #[must_use]
    pub fn profile_id(&self) -> &str {
        match self {
            Self::Qubo(model) => &model.profile_id,
            Self::Ising(model) => &model.profile_id,
        }
    }

    /// Return the number of logical binary variables or spins.
    #[must_use]
    pub fn variable_count(&self) -> usize {
        match self {
            Self::Qubo(model) => model.variables.len(),
            Self::Ising(model) => model.linear.len(),
        }
    }

    /// Canonically hash the underlying QUBO or Ising model.
    pub fn model_sha256(&self) -> Result<Sha256Digest, QuantumContractError> {
        Ok(match self {
            Self::Qubo(model) => hash_value(model)?,
            Self::Ising(model) => hash_value(model)?,
        })
    }

    /// Validate schema, dimensions, indexes, ordering, and finite coefficients.
    pub fn validate(&self) -> Result<(), QuantumContractError> {
        match self {
            Self::Qubo(model) => validate_qubo(model),
            Self::Ising(model) => validate_ising(model),
        }
    }

    /// Evaluate a normalized binary assignment in the approved logical model.
    ///
    /// For an Ising model, each binary value is converted with `z = 1 - 2x`.
    pub fn energy(&self, assignment: &[bool]) -> Result<f64, QuantumContractError> {
        self.validate()?;
        if assignment.len() != self.variable_count() {
            return Err(QuantumContractError::InvalidContract(format!(
                "expected {} binary values, received {}",
                self.variable_count(),
                assignment.len()
            )));
        }
        Ok(match self {
            Self::Qubo(model) => model.energy(assignment)?,
            Self::Ising(model) => {
                let spins: Vec<_> = assignment
                    .iter()
                    .map(|selected| if *selected { -1 } else { 1 })
                    .collect();
                model.energy(&spins)?
            }
        })
    }
}

fn validate_qubo(model: &QuboModel) -> Result<(), QuantumContractError> {
    require_schema("QUBO model", &model.schema_version, QUBO_SCHEMA_VERSION)?;
    require_nonempty("QUBO profile_id", &model.profile_id)?;
    if model.linear.len() != model.variables.len() {
        return Err(QuantumContractError::InvalidContract(format!(
            "QUBO has {} variables but {} linear coefficients",
            model.variables.len(),
            model.linear.len()
        )));
    }
    for (expected, variable) in model.variables.iter().enumerate() {
        if variable.index != expected {
            return Err(QuantumContractError::InvalidContract(format!(
                "QUBO variable position {expected} declares index {}",
                variable.index
            )));
        }
        require_nonempty("QUBO variable name", &variable.name)?;
        require_nonempty("QUBO variable source_id", &variable.source_id)?;
    }
    validate_finite(
        model
            .linear
            .iter()
            .copied()
            .chain([model.offset, model.constraint_penalty]),
        "QUBO coefficient",
    )?;
    if model.constraint_penalty <= 0.0 {
        return Err(QuantumContractError::InvalidContract(
            "QUBO constraint_penalty must be strictly positive".to_owned(),
        ));
    }
    if model.request.max_inputs == Some(0)
        || model
            .request
            .coverage_floor
            .is_some_and(|floor| !(floor.is_finite() && 0.0 < floor && floor <= 1.0))
    {
        return Err(QuantumContractError::InvalidContract(
            "QUBO optimization request contains an invalid bound".to_owned(),
        ));
    }
    let mut previous = None;
    for coupling in &model.quadratic {
        if coupling.left >= coupling.right || coupling.right >= model.variables.len() {
            return Err(QuantumContractError::InvalidContract(format!(
                "invalid QUBO coupling ({}, {}) for {} variables",
                coupling.left,
                coupling.right,
                model.variables.len()
            )));
        }
        let key = (coupling.left, coupling.right);
        if previous.is_some_and(|prior| prior >= key) {
            return Err(QuantumContractError::InvalidContract(
                "QUBO couplings must be strictly sorted and unique".to_owned(),
            ));
        }
        if !coupling.coefficient.is_finite() {
            return Err(QuantumContractError::InvalidContract(
                "QUBO coupling coefficient must be finite".to_owned(),
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

fn validate_ising(model: &IsingModel) -> Result<(), QuantumContractError> {
    require_schema(
        "Ising model",
        &model.schema_version,
        qbm_benchmark::ISING_SCHEMA_VERSION,
    )?;
    require_nonempty("Ising profile_id", &model.profile_id)?;
    validate_finite(
        model.linear.iter().copied().chain([model.offset]),
        "Ising coefficient",
    )?;
    let mut previous = None;
    for coupling in &model.quadratic {
        if coupling.left >= coupling.right || coupling.right >= model.linear.len() {
            return Err(QuantumContractError::InvalidContract(format!(
                "invalid Ising coupling ({}, {}) for {} spins",
                coupling.left,
                coupling.right,
                model.linear.len()
            )));
        }
        let key = (coupling.left, coupling.right);
        if previous.is_some_and(|prior| prior >= key) {
            return Err(QuantumContractError::InvalidContract(
                "Ising couplings must be strictly sorted and unique".to_owned(),
            ));
        }
        if !coupling.coefficient.is_finite() {
            return Err(QuantumContractError::InvalidContract(
                "Ising coupling coefficient must be finite".to_owned(),
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

/// How executor-returned bits map to logical QUBO and Ising values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryAssignmentConvention {
    /// Bits are QUBO `x`; Ising spins are recovered by `z = 1 - 2x`.
    QuboX,
}

/// One logical variable that selects a native biomedical input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputVariableBinding {
    /// Zero-based position in every returned binary assignment.
    pub variable_index: usize,
    /// Stable input identity in the benchmark profile.
    pub input_id: String,
}

/// Immutable mapping needed to rescore logical samples in native profile units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRescoringMetadata {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Stable profile identity.
    pub profile_id: String,
    /// Canonical identity of the complete benchmark profile.
    pub profile_sha256: Sha256Digest,
    /// Canonical identity of the QUBO that defines input-variable semantics.
    pub source_qubo_sha256: Sha256Digest,
    /// Number of values expected in a returned assignment.
    pub variable_count: usize,
    /// Ordered bindings for variables with native input semantics.
    pub input_variables: Vec<InputVariableBinding>,
    /// Normalized assignment representation returned by executors.
    pub assignment_convention: BinaryAssignmentConvention,
}

impl NativeRescoringMetadata {
    /// Derive a native-rescoring map from an exact QUBO and benchmark profile.
    pub fn from_qubo(
        profile: &BenchmarkProfile,
        qubo: &QuboModel,
    ) -> Result<Self, QuantumContractError> {
        profile.validate()?;
        validate_qubo(qubo)?;
        if profile.profile_id != qubo.profile_id {
            return Err(QuantumContractError::InvalidContract(format!(
                "profile {:?} does not match QUBO profile {:?}",
                profile.profile_id, qubo.profile_id
            )));
        }
        let metadata = Self {
            schema_version: NATIVE_RESCORING_METADATA_SCHEMA_VERSION.to_owned(),
            profile_id: profile.profile_id.clone(),
            profile_sha256: hash_value(profile)?,
            source_qubo_sha256: hash_value(qubo)?,
            variable_count: qubo.variables.len(),
            input_variables: qubo
                .variables
                .iter()
                .filter(|variable| variable.kind == QuboVariableKind::Input)
                .map(|variable| InputVariableBinding {
                    variable_index: variable.index,
                    input_id: variable.source_id.clone(),
                })
                .collect(),
            assignment_convention: BinaryAssignmentConvention::QuboX,
        };
        metadata.validate_for_problem(&QuantumProblem::Qubo(qubo.clone()))?;
        metadata.validate_profile(profile)?;
        Ok(metadata)
    }

    /// Validate this mapping against an executable logical problem.
    pub fn validate_for_problem(
        &self,
        problem: &QuantumProblem,
    ) -> Result<(), QuantumContractError> {
        require_schema(
            "native rescoring metadata",
            &self.schema_version,
            NATIVE_RESCORING_METADATA_SCHEMA_VERSION,
        )?;
        problem.validate()?;
        if self.profile_id != problem.profile_id() {
            return Err(QuantumContractError::InvalidContract(format!(
                "rescoring profile {:?} does not match problem profile {:?}",
                self.profile_id,
                problem.profile_id()
            )));
        }
        if self.variable_count != problem.variable_count() {
            return Err(QuantumContractError::InvalidContract(format!(
                "rescoring metadata expects {} variables but problem has {}",
                self.variable_count,
                problem.variable_count()
            )));
        }
        let mut previous = None;
        let mut input_ids = BTreeSet::new();
        for binding in &self.input_variables {
            if binding.variable_index >= self.variable_count {
                return Err(QuantumContractError::InvalidContract(format!(
                    "native input variable {} is outside assignment length {}",
                    binding.variable_index, self.variable_count
                )));
            }
            if previous.is_some_and(|prior| prior >= binding.variable_index) {
                return Err(QuantumContractError::InvalidContract(
                    "native input bindings must be strictly ordered by variable_index".to_owned(),
                ));
            }
            require_nonempty("native input_id", &binding.input_id)?;
            if !input_ids.insert(binding.input_id.as_str()) {
                return Err(QuantumContractError::InvalidContract(format!(
                    "duplicate native input binding {:?}",
                    binding.input_id
                )));
            }
            previous = Some(binding.variable_index);
        }
        if let QuantumProblem::Qubo(qubo) = problem {
            let computed_qubo = hash_value(qubo)?;
            if self.source_qubo_sha256 != computed_qubo {
                return Err(QuantumContractError::IdentityMismatch {
                    contract: "native rescoring source QUBO",
                    declared: self.source_qubo_sha256.clone(),
                    computed: computed_qubo,
                });
            }
            let expected: Vec<_> = qubo
                .variables
                .iter()
                .filter(|variable| variable.kind == QuboVariableKind::Input)
                .map(|variable| InputVariableBinding {
                    variable_index: variable.index,
                    input_id: variable.source_id.clone(),
                })
                .collect();
            if self.input_variables != expected {
                return Err(QuantumContractError::InvalidContract(
                    "native input bindings do not match QUBO variable semantics".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Validate the immutable profile identity and complete input mapping.
    pub fn validate_profile(&self, profile: &BenchmarkProfile) -> Result<(), QuantumContractError> {
        profile.validate()?;
        if self.profile_id != profile.profile_id {
            return Err(QuantumContractError::InvalidContract(format!(
                "rescoring metadata names profile {:?}, received {:?}",
                self.profile_id, profile.profile_id
            )));
        }
        let computed = hash_value(profile)?;
        if self.profile_sha256 != computed {
            return Err(QuantumContractError::IdentityMismatch {
                contract: "native rescoring profile",
                declared: self.profile_sha256.clone(),
                computed,
            });
        }
        let declared: BTreeSet<_> = self
            .input_variables
            .iter()
            .map(|binding| binding.input_id.as_str())
            .collect();
        let expected: BTreeSet<_> = profile
            .inputs
            .iter()
            .map(|input| input.id.as_str())
            .collect();
        if declared != expected {
            return Err(QuantumContractError::InvalidContract(
                "native input bindings do not cover the profile inputs exactly".to_owned(),
            ));
        }
        Ok(())
    }

    /// Rescore one normalized assignment with the benchmark's native objective.
    pub fn rescore(
        &self,
        profile: &BenchmarkProfile,
        assignment: &[bool],
    ) -> Result<SelectionScore, QuantumContractError> {
        self.validate_profile(profile)?;
        if assignment.len() != self.variable_count {
            return Err(QuantumContractError::InvalidContract(format!(
                "expected {} binary values for native rescoring, received {}",
                self.variable_count,
                assignment.len()
            )));
        }
        let selected: Vec<_> = self
            .input_variables
            .iter()
            .filter(|binding| assignment[binding.variable_index])
            .map(|binding| binding.input_id.clone())
            .collect();
        Ok(evaluate_selection(profile, &selected)?)
    }
}

/// Exact manual approval lineage for a serialized logical model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalProvenance {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Approval bound to the exact canonical model digest.
    pub approval: Approval,
    /// Canonical SHA-256 of the underlying QUBO or Ising model.
    pub source_model_sha256: Sha256Digest,
}

impl ApprovalProvenance {
    fn new(problem: &QuantumProblem, approval: Approval) -> Result<Self, QuantumContractError> {
        let provenance = Self {
            schema_version: APPROVAL_PROVENANCE_SCHEMA_VERSION.to_owned(),
            source_model_sha256: problem.model_sha256()?,
            approval,
        };
        provenance.validate(problem)?;
        Ok(provenance)
    }

    /// Ensure the decision approved the exact canonical logical model.
    pub fn validate(&self, problem: &QuantumProblem) -> Result<(), QuantumContractError> {
        require_schema(
            "approval provenance",
            &self.schema_version,
            APPROVAL_PROVENANCE_SCHEMA_VERSION,
        )?;
        if self.approval.decision != Decision::Approve {
            return Err(QuantumContractError::InvalidContract(
                "only an approve decision can authorize an execution payload".to_owned(),
            ));
        }
        let computed = problem.model_sha256()?;
        if self.source_model_sha256 != computed {
            return Err(QuantumContractError::IdentityMismatch {
                contract: "approved source model",
                declared: self.source_model_sha256.clone(),
                computed,
            });
        }
        if self.approval.stage_output_hash != self.source_model_sha256 {
            return Err(QuantumContractError::InvalidContract(
                "approval is not bound to the exact canonical QUBO/Ising model".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Content-addressed, approved QUBO/Ising payload that can be exported safely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedProblemPayload {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Canonical identity of all fields other than this digest.
    pub payload_sha256: Sha256Digest,
    /// Exact approved logical model.
    pub problem: QuantumProblem,
    /// Exact approval and source-model digest.
    pub provenance: ApprovalProvenance,
    /// Mapping required for native biomedical rescoring.
    pub native_rescoring: NativeRescoringMetadata,
}

#[derive(Serialize)]
struct PayloadIdentity<'a> {
    schema_version: &'a str,
    problem: &'a QuantumProblem,
    provenance: &'a ApprovalProvenance,
    native_rescoring: &'a NativeRescoringMetadata,
}

impl ApprovedProblemPayload {
    /// Construct an approved QUBO payload and derive its native input mapping.
    pub fn from_qubo(
        qubo: QuboModel,
        profile: &BenchmarkProfile,
        approval: Approval,
    ) -> Result<Self, QuantumContractError> {
        let metadata = NativeRescoringMetadata::from_qubo(profile, &qubo)?;
        Self::new(QuantumProblem::Qubo(qubo), metadata, approval)
    }

    /// Construct an approved Ising payload with an explicitly reviewed mapping.
    pub fn from_ising(
        ising: IsingModel,
        native_rescoring: NativeRescoringMetadata,
        approval: Approval,
    ) -> Result<Self, QuantumContractError> {
        Self::new(QuantumProblem::Ising(ising), native_rescoring, approval)
    }

    /// Construct an Ising payload derived from a QUBO after bounded energy checks.
    ///
    /// The returned report records whether checking was exhaustive. The approval
    /// must still be bound to the exact Ising model being placed in the payload.
    pub fn from_equivalent_ising(
        qubo: &QuboModel,
        ising: IsingModel,
        profile: &BenchmarkProfile,
        approval: Approval,
        config: crate::EnergyEquivalenceConfig,
    ) -> Result<(Self, crate::EnergyEquivalenceReport), QuantumContractError> {
        let report = crate::validate_energy_equivalence(qubo, &ising, config)?;
        let native_rescoring = NativeRescoringMetadata::from_qubo(profile, qubo)?;
        let payload = Self::from_ising(ising, native_rescoring, approval)?;
        Ok((payload, report))
    }

    /// Construct a content-addressed payload after checking exact approval.
    pub fn new(
        problem: QuantumProblem,
        native_rescoring: NativeRescoringMetadata,
        approval: Approval,
    ) -> Result<Self, QuantumContractError> {
        problem.validate()?;
        native_rescoring.validate_for_problem(&problem)?;
        let provenance = ApprovalProvenance::new(&problem, approval)?;
        let payload_sha256 = hash_value(&PayloadIdentity {
            schema_version: APPROVED_PAYLOAD_SCHEMA_VERSION,
            problem: &problem,
            provenance: &provenance,
            native_rescoring: &native_rescoring,
        })?;
        Ok(Self {
            schema_version: APPROVED_PAYLOAD_SCHEMA_VERSION.to_owned(),
            payload_sha256,
            problem,
            provenance,
            native_rescoring,
        })
    }

    /// Validate approval, model structure, native mapping, and payload identity.
    pub fn validate(&self) -> Result<(), QuantumContractError> {
        require_schema(
            "approved quantum payload",
            &self.schema_version,
            APPROVED_PAYLOAD_SCHEMA_VERSION,
        )?;
        self.problem.validate()?;
        self.provenance.validate(&self.problem)?;
        self.native_rescoring.validate_for_problem(&self.problem)?;
        let computed = hash_value(&PayloadIdentity {
            schema_version: &self.schema_version,
            problem: &self.problem,
            provenance: &self.provenance,
            native_rescoring: &self.native_rescoring,
        })?;
        if self.payload_sha256 != computed {
            return Err(QuantumContractError::IdentityMismatch {
                contract: "approved quantum payload",
                declared: self.payload_sha256.clone(),
                computed,
            });
        }
        Ok(())
    }
}

/// General category of a provider-neutral execution target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// Classical execution of the logical quantum model.
    Simulator,
    /// Quantum processing hardware.
    QuantumProcessor,
    /// Provider-managed hybrid classical/quantum execution.
    Hybrid,
}

/// Public, non-secret capability report for one executor backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendCapability {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Stable backend identifier understood by its executor implementation.
    pub backend_id: String,
    /// Safe provider label for reports; this is not an endpoint.
    pub provider: String,
    /// Human-readable backend name.
    pub display_name: String,
    /// General execution target category.
    pub backend_kind: BackendKind,
    /// Strictly sorted, unique logical encodings accepted by the backend.
    pub supported_encodings: Vec<ProblemEncoding>,
    /// Optional declared logical-variable ceiling before embedding.
    pub maximum_variables: Option<usize>,
    /// Optional declared logical-coupling ceiling before embedding.
    pub maximum_couplings: Option<usize>,
    /// Optional declared sample-count ceiling per request.
    pub maximum_samples_per_request: Option<u32>,
    /// Whether the backend accepts a caller-provided sampling seed.
    pub supports_seed: bool,
    /// Whether jobs can be polled after submission.
    pub supports_polling: bool,
    /// Deterministically ordered, safe-to-display limitations.
    pub limitations: Vec<String>,
}

impl BackendCapability {
    /// Validate display fields, ordered encodings, and positive optional limits.
    pub fn validate(&self) -> Result<(), QuantumContractError> {
        require_schema(
            "backend capability",
            &self.schema_version,
            BACKEND_CAPABILITY_SCHEMA_VERSION,
        )?;
        require_safe_text("backend_id", &self.backend_id)?;
        require_safe_text("provider", &self.provider)?;
        require_safe_text("display_name", &self.display_name)?;
        if self.supported_encodings.is_empty()
            || !self
                .supported_encodings
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return Err(QuantumContractError::InvalidContract(
                "supported_encodings must be non-empty, strictly sorted, and unique".to_owned(),
            ));
        }
        if self.maximum_variables == Some(0) {
            return Err(QuantumContractError::InvalidContract(
                "maximum_variables must be positive when declared".to_owned(),
            ));
        }
        if self.maximum_couplings == Some(0) {
            return Err(QuantumContractError::InvalidContract(
                "maximum_couplings must be positive when declared".to_owned(),
            ));
        }
        if self.maximum_samples_per_request == Some(0) {
            return Err(QuantumContractError::InvalidContract(
                "maximum_samples_per_request must be positive when declared".to_owned(),
            ));
        }
        validate_report_lines(&self.limitations, "backend limitations")
    }
}

/// Browser-safe availability state for optional execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    /// A configured executor reports itself ready for submission.
    Ready,
    /// Payload export and local validation are available, execution is not.
    ExportOnly,
    /// No executor has been configured.
    NotConfigured,
}

/// Versioned readiness result suitable for API and browser reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReadiness {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Availability state.
    pub status: ReadinessStatus,
    /// Safe, user-facing explanation.
    pub summary: String,
    /// Optional public backend capabilities; never configuration or credentials.
    pub backend: Option<BackendCapability>,
    /// Deterministically ordered, safe-to-display limitations.
    pub limitations: Vec<String>,
}

impl ExecutionReadiness {
    /// Report a configured backend as ready.
    #[must_use]
    pub fn ready(backend: BackendCapability) -> Self {
        Self {
            schema_version: EXECUTION_READINESS_SCHEMA_VERSION.to_owned(),
            status: ReadinessStatus::Ready,
            summary: format!(
                "{} is configured for optional execution",
                backend.display_name
            ),
            backend: Some(backend),
            limitations: vec![NO_QUANTUM_ADVANTAGE_CLAIM.to_owned()],
        }
    }

    /// Report that approved payload export is available without execution.
    #[must_use]
    pub fn export_only() -> Self {
        Self {
            schema_version: EXECUTION_READINESS_SCHEMA_VERSION.to_owned(),
            status: ReadinessStatus::ExportOnly,
            summary: "approved QUBO/Ising payloads can be exported; no executor is active"
                .to_owned(),
            backend: None,
            limitations: vec![NO_QUANTUM_ADVANTAGE_CLAIM.to_owned()],
        }
    }

    /// Report that execution has not been configured.
    #[must_use]
    pub fn not_configured() -> Self {
        Self {
            schema_version: EXECUTION_READINESS_SCHEMA_VERSION.to_owned(),
            status: ReadinessStatus::NotConfigured,
            summary: "quantum execution is not configured".to_owned(),
            backend: None,
            limitations: vec![
                "no provider request was attempted".to_owned(),
                NO_QUANTUM_ADVANTAGE_CLAIM.to_owned(),
            ],
        }
    }

    /// Validate this browser-safe readiness report.
    pub fn validate(&self) -> Result<(), QuantumContractError> {
        require_schema(
            "execution readiness",
            &self.schema_version,
            EXECUTION_READINESS_SCHEMA_VERSION,
        )?;
        require_safe_text("readiness summary", &self.summary)?;
        if self.status == ReadinessStatus::Ready && self.backend.is_none() {
            return Err(QuantumContractError::InvalidContract(
                "ready status requires backend capabilities".to_owned(),
            ));
        }
        if let Some(backend) = &self.backend {
            backend.validate()?;
        }
        validate_report_lines(&self.limitations, "readiness limitations")
    }
}

/// Immutable request to sample one exact approved logical payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Canonical identity of all fields other than this digest.
    pub request_sha256: Sha256Digest,
    /// Approved model and approval lineage submitted to the executor.
    pub payload: ApprovedProblemPayload,
    /// Target backend identifier.
    pub backend_id: String,
    /// Requested number of samples.
    pub sample_count: u32,
    /// Optional reproducibility seed when the backend supports one.
    pub seed: Option<u64>,
    /// Optional caller-side wall-time ceiling in seconds.
    pub maximum_runtime_seconds: Option<u64>,
}

#[derive(Serialize)]
struct RequestIdentity<'a> {
    schema_version: &'a str,
    payload: &'a ApprovedProblemPayload,
    backend_id: &'a str,
    sample_count: u32,
    seed: Option<u64>,
    maximum_runtime_seconds: Option<u64>,
}

impl ExecutionRequest {
    /// Construct and content-address a request with no seed or runtime ceiling.
    pub fn new(
        payload: ApprovedProblemPayload,
        backend_id: impl Into<String>,
        sample_count: u32,
    ) -> Result<Self, QuantumContractError> {
        let mut request = Self {
            schema_version: EXECUTION_REQUEST_SCHEMA_VERSION.to_owned(),
            request_sha256: payload.payload_sha256.clone(),
            payload,
            backend_id: backend_id.into(),
            sample_count,
            seed: None,
            maximum_runtime_seconds: None,
        };
        request.refresh_identity()?;
        request.validate()?;
        Ok(request)
    }

    /// Return a copy with an explicit sampling seed and refreshed identity.
    pub fn with_seed(mut self, seed: u64) -> Result<Self, QuantumContractError> {
        self.seed = Some(seed);
        self.refresh_identity()?;
        Ok(self)
    }

    /// Return a copy with a positive runtime ceiling and refreshed identity.
    pub fn with_maximum_runtime_seconds(
        mut self,
        seconds: u64,
    ) -> Result<Self, QuantumContractError> {
        self.maximum_runtime_seconds = Some(seconds);
        self.refresh_identity()?;
        self.validate()?;
        Ok(self)
    }

    fn identity(&self) -> RequestIdentity<'_> {
        RequestIdentity {
            schema_version: &self.schema_version,
            payload: &self.payload,
            backend_id: &self.backend_id,
            sample_count: self.sample_count,
            seed: self.seed,
            maximum_runtime_seconds: self.maximum_runtime_seconds,
        }
    }

    fn refresh_identity(&mut self) -> Result<(), QuantumContractError> {
        self.request_sha256 = hash_value(&self.identity())?;
        Ok(())
    }

    /// Validate request structure, approval lineage, and content identity.
    pub fn validate(&self) -> Result<(), QuantumContractError> {
        require_schema(
            "execution request",
            &self.schema_version,
            EXECUTION_REQUEST_SCHEMA_VERSION,
        )?;
        self.payload.validate()?;
        require_safe_text("backend_id", &self.backend_id)?;
        if self.sample_count == 0 {
            return Err(QuantumContractError::InvalidContract(
                "sample_count must be positive".to_owned(),
            ));
        }
        if self.maximum_runtime_seconds == Some(0) {
            return Err(QuantumContractError::InvalidContract(
                "maximum_runtime_seconds must be positive when declared".to_owned(),
            ));
        }
        let computed = hash_value(&self.identity())?;
        if self.request_sha256 != computed {
            return Err(QuantumContractError::IdentityMismatch {
                contract: "execution request",
                declared: self.request_sha256.clone(),
                computed,
            });
        }
        Ok(())
    }

    /// Validate this request against public backend capabilities.
    pub fn validate_for_backend(
        &self,
        backend: &BackendCapability,
    ) -> Result<(), QuantumContractError> {
        self.validate()?;
        backend.validate()?;
        if self.backend_id != backend.backend_id {
            return Err(QuantumContractError::InvalidContract(format!(
                "request backend {:?} does not match capability backend {:?}",
                self.backend_id, backend.backend_id
            )));
        }
        if !backend
            .supported_encodings
            .contains(&self.payload.problem.encoding())
        {
            return Err(QuantumContractError::InvalidContract(format!(
                "backend {:?} does not declare {:?} support",
                backend.backend_id,
                self.payload.problem.encoding()
            )));
        }
        if backend
            .maximum_variables
            .is_some_and(|maximum| self.payload.problem.variable_count() > maximum)
        {
            return Err(QuantumContractError::InvalidContract(format!(
                "problem has {} variables, above backend limit {}",
                self.payload.problem.variable_count(),
                backend.maximum_variables.unwrap_or_default()
            )));
        }
        let coupling_count = match &self.payload.problem {
            QuantumProblem::Qubo(model) => model.quadratic.len(),
            QuantumProblem::Ising(model) => model.quadratic.len(),
        };
        if backend
            .maximum_couplings
            .is_some_and(|maximum| coupling_count > maximum)
        {
            return Err(QuantumContractError::InvalidContract(format!(
                "problem has {coupling_count} couplings, above backend limit {}",
                backend.maximum_couplings.unwrap_or_default()
            )));
        }
        if backend
            .maximum_samples_per_request
            .is_some_and(|maximum| self.sample_count > maximum)
        {
            return Err(QuantumContractError::InvalidContract(format!(
                "request asks for {} samples, above backend limit {}",
                self.sample_count,
                backend.maximum_samples_per_request.unwrap_or_default()
            )));
        }
        if self.seed.is_some() && !backend.supports_seed {
            return Err(QuantumContractError::InvalidContract(format!(
                "backend {:?} does not declare seed support",
                backend.backend_id
            )));
        }
        Ok(())
    }
}

/// Provider-neutral lifecycle state for a submitted job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// The executor accepted the immutable request.
    Submitted,
    /// Execution is in progress.
    Running,
    /// Execution completed with samples.
    Succeeded,
    /// Execution ended with a sanitized failure summary.
    Failed,
    /// Execution was cancelled.
    Cancelled,
}

impl JobState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Immutable receipt linking a provider job to one exact execution request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobReceipt {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Canonical identity of all fields other than this digest.
    pub receipt_sha256: Sha256Digest,
    /// Exact request accepted by the executor.
    pub request_sha256: Sha256Digest,
    /// Exact approved payload accepted by the executor.
    pub payload_sha256: Sha256Digest,
    /// Backend that accepted the request.
    pub backend_id: String,
    /// Opaque provider job identifier, never a URL or credential.
    pub job_id: String,
    /// Observed lifecycle state.
    pub state: JobState,
    /// Time the executor accepted the request.
    pub submitted_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct ReceiptIdentity<'a> {
    schema_version: &'a str,
    request_sha256: &'a Sha256Digest,
    payload_sha256: &'a Sha256Digest,
    backend_id: &'a str,
    job_id: &'a str,
    state: JobState,
    submitted_at: &'a DateTime<Utc>,
}

impl JobReceipt {
    /// Construct and content-address a receipt returned by an executor.
    pub fn new(
        request: &ExecutionRequest,
        job_id: impl Into<String>,
        state: JobState,
        submitted_at: DateTime<Utc>,
    ) -> Result<Self, QuantumContractError> {
        request.validate()?;
        let mut receipt = Self {
            schema_version: JOB_RECEIPT_SCHEMA_VERSION.to_owned(),
            receipt_sha256: request.request_sha256.clone(),
            request_sha256: request.request_sha256.clone(),
            payload_sha256: request.payload.payload_sha256.clone(),
            backend_id: request.backend_id.clone(),
            job_id: job_id.into(),
            state,
            submitted_at,
        };
        receipt.receipt_sha256 = hash_value(&receipt.identity())?;
        receipt.validate_against(request)?;
        Ok(receipt)
    }

    fn identity(&self) -> ReceiptIdentity<'_> {
        ReceiptIdentity {
            schema_version: &self.schema_version,
            request_sha256: &self.request_sha256,
            payload_sha256: &self.payload_sha256,
            backend_id: &self.backend_id,
            job_id: &self.job_id,
            state: self.state,
            submitted_at: &self.submitted_at,
        }
    }

    /// Validate linkage to an immutable request and this receipt's identity.
    pub fn validate_against(&self, request: &ExecutionRequest) -> Result<(), QuantumContractError> {
        require_schema(
            "job receipt",
            &self.schema_version,
            JOB_RECEIPT_SCHEMA_VERSION,
        )?;
        request.validate()?;
        require_safe_text("job_id", &self.job_id)?;
        if self.request_sha256 != request.request_sha256
            || self.payload_sha256 != request.payload.payload_sha256
            || self.backend_id != request.backend_id
        {
            return Err(QuantumContractError::InvalidContract(
                "job receipt is not linked to the supplied execution request".to_owned(),
            ));
        }
        let computed = hash_value(&self.identity())?;
        if self.receipt_sha256 != computed {
            return Err(QuantumContractError::IdentityMismatch {
                contract: "job receipt",
                declared: self.receipt_sha256.clone(),
                computed,
            });
        }
        Ok(())
    }
}

/// Sanitized provider failure suitable for persistence and browser display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobFailure {
    /// Stable executor-defined error category, not provider response content.
    pub code: String,
    /// Safe human-readable summary with no credentials or request headers.
    pub summary: String,
    /// Whether retrying the exact request may succeed.
    pub retryable: bool,
}

impl JobFailure {
    fn validate(&self) -> Result<(), QuantumContractError> {
        require_safe_text("job failure code", &self.code)?;
        require_safe_text("job failure summary", &self.summary)
    }
}

/// One normalized logical sample returned by an executor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSample {
    /// QUBO `x` values; Ising samples use `x = (1-z)/2`.
    pub binary_assignment: Vec<bool>,
    /// Energy recomputed in the exact approved logical model.
    pub logical_energy: f64,
    /// Optional finite energy reported by the backend before local checking.
    pub backend_reported_energy: Option<f64>,
    /// Number of observations represented by this unique assignment.
    pub occurrences: u64,
}

impl ExecutionSample {
    /// Construct a sample and compute its trusted logical energy locally.
    pub fn new(
        problem: &QuantumProblem,
        binary_assignment: Vec<bool>,
        backend_reported_energy: Option<f64>,
        occurrences: u64,
    ) -> Result<Self, QuantumContractError> {
        let sample = Self {
            logical_energy: problem.energy(&binary_assignment)?,
            binary_assignment,
            backend_reported_energy,
            occurrences,
        };
        sample.validate_for(problem)?;
        Ok(sample)
    }

    fn validate_for(&self, problem: &QuantumProblem) -> Result<(), QuantumContractError> {
        if self.occurrences == 0 {
            return Err(QuantumContractError::InvalidContract(
                "sample occurrences must be positive".to_owned(),
            ));
        }
        if !self.logical_energy.is_finite()
            || self
                .backend_reported_energy
                .is_some_and(|energy| !energy.is_finite())
        {
            return Err(QuantumContractError::InvalidContract(
                "sample energies must be finite".to_owned(),
            ));
        }
        let expected = problem.energy(&self.binary_assignment)?;
        let allowed = 1.0e-10 + 1.0e-12 * expected.abs().max(self.logical_energy.abs());
        if (expected - self.logical_energy).abs() > allowed {
            return Err(QuantumContractError::InvalidContract(format!(
                "sample logical energy {} does not match locally computed energy {expected}",
                self.logical_energy
            )));
        }
        Ok(())
    }
}

/// Terminal outcome of an execution job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    /// Samples were returned and validated structurally.
    Succeeded,
    /// Execution failed with a sanitized explanation.
    Failed,
    /// Execution was cancelled.
    Cancelled,
}

impl JobOutcome {
    const fn receipt_state(self) -> JobState {
        match self {
            Self::Succeeded => JobState::Succeeded,
            Self::Failed => JobState::Failed,
            Self::Cancelled => JobState::Cancelled,
        }
    }
}

/// Native score for one normalized result sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRescoredSample {
    /// Zero-based sample position in the enclosing result.
    pub sample_index: usize,
    /// Constraint-aware score in benchmark-profile units.
    pub score: SelectionScore,
}

/// Deterministic native rescoring attached after logical sample validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRescoringReport {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Canonical identity of the mapping used for rescoring.
    pub metadata_sha256: Sha256Digest,
    /// Exact benchmark profile used for native scoring.
    pub profile_sha256: Sha256Digest,
    /// Per-sample scores in result order.
    pub samples: Vec<NativeRescoredSample>,
    /// Mandatory interpretation boundary.
    pub disclaimer: String,
}

impl NativeRescoringReport {
    fn build(
        metadata: &NativeRescoringMetadata,
        samples: &[ExecutionSample],
        profile: &BenchmarkProfile,
    ) -> Result<Self, QuantumContractError> {
        metadata.validate_profile(profile)?;
        let report = Self {
            schema_version: NATIVE_RESCORING_REPORT_SCHEMA_VERSION.to_owned(),
            metadata_sha256: hash_value(metadata)?,
            profile_sha256: metadata.profile_sha256.clone(),
            samples: samples
                .iter()
                .enumerate()
                .map(|(sample_index, sample)| {
                    Ok(NativeRescoredSample {
                        sample_index,
                        score: metadata.rescore(profile, &sample.binary_assignment)?,
                    })
                })
                .collect::<Result<_, QuantumContractError>>()?,
            disclaimer: NO_QUANTUM_ADVANTAGE_CLAIM.to_owned(),
        };
        report.validate(metadata, samples, profile)?;
        Ok(report)
    }

    fn validate(
        &self,
        metadata: &NativeRescoringMetadata,
        samples: &[ExecutionSample],
        profile: &BenchmarkProfile,
    ) -> Result<(), QuantumContractError> {
        require_schema(
            "native rescoring report",
            &self.schema_version,
            NATIVE_RESCORING_REPORT_SCHEMA_VERSION,
        )?;
        metadata.validate_profile(profile)?;
        let expected_metadata = hash_value(metadata)?;
        if self.metadata_sha256 != expected_metadata
            || self.profile_sha256 != metadata.profile_sha256
        {
            return Err(QuantumContractError::InvalidContract(
                "native rescoring report is not linked to the supplied metadata/profile".to_owned(),
            ));
        }
        if self.samples.len() != samples.len() {
            return Err(QuantumContractError::InvalidContract(
                "native rescoring report must score every result sample exactly once".to_owned(),
            ));
        }
        for (sample_index, (reported, sample)) in self.samples.iter().zip(samples).enumerate() {
            if reported.sample_index != sample_index
                || reported.score != metadata.rescore(profile, &sample.binary_assignment)?
            {
                return Err(QuantumContractError::InvalidContract(format!(
                    "native rescore at sample {sample_index} does not match deterministic scoring"
                )));
            }
        }
        if self.disclaimer != NO_QUANTUM_ADVANTAGE_CLAIM {
            return Err(QuantumContractError::InvalidContract(
                "native rescoring report is missing its interpretation boundary".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Immutable terminal result for one exact request and job receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobResult {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Canonical identity of all fields other than this digest.
    pub result_sha256: Sha256Digest,
    /// Terminal receipt associated with this result.
    pub receipt_sha256: Sha256Digest,
    /// Exact submitted request.
    pub request_sha256: Sha256Digest,
    /// Exact approved payload.
    pub payload_sha256: Sha256Digest,
    /// Backend that produced the terminal outcome.
    pub backend_id: String,
    /// Opaque provider job identifier.
    pub job_id: String,
    /// Terminal outcome.
    pub outcome: JobOutcome,
    /// Unique normalized logical samples, sorted by energy then assignment.
    pub samples: Vec<ExecutionSample>,
    /// Sanitized failure details for unsuccessful outcomes.
    pub failure: Option<JobFailure>,
    /// Optional deterministic native rescoring performed by the host.
    pub native_rescoring: Option<NativeRescoringReport>,
    /// Time the terminal outcome was observed.
    pub completed_at: DateTime<Utc>,
    /// Deterministically ordered interpretation boundaries.
    pub limitations: Vec<String>,
}

#[derive(Serialize)]
struct ResultIdentity<'a> {
    schema_version: &'a str,
    receipt_sha256: &'a Sha256Digest,
    request_sha256: &'a Sha256Digest,
    payload_sha256: &'a Sha256Digest,
    backend_id: &'a str,
    job_id: &'a str,
    outcome: JobOutcome,
    samples: &'a [ExecutionSample],
    failure: &'a Option<JobFailure>,
    native_rescoring: &'a Option<NativeRescoringReport>,
    completed_at: &'a DateTime<Utc>,
    limitations: &'a [String],
}

impl JobResult {
    /// Construct a successful result from a terminal receipt and logical samples.
    pub fn succeeded(
        request: &ExecutionRequest,
        receipt: &JobReceipt,
        samples: Vec<ExecutionSample>,
        completed_at: DateTime<Utc>,
    ) -> Result<Self, QuantumContractError> {
        Self::build(
            request,
            receipt,
            JobOutcome::Succeeded,
            samples,
            None,
            completed_at,
        )
    }

    /// Construct a failed result with no sample data.
    pub fn failed(
        request: &ExecutionRequest,
        receipt: &JobReceipt,
        failure: JobFailure,
        completed_at: DateTime<Utc>,
    ) -> Result<Self, QuantumContractError> {
        Self::build(
            request,
            receipt,
            JobOutcome::Failed,
            Vec::new(),
            Some(failure),
            completed_at,
        )
    }

    /// Construct a cancelled result with no sample data.
    pub fn cancelled(
        request: &ExecutionRequest,
        receipt: &JobReceipt,
        completed_at: DateTime<Utc>,
    ) -> Result<Self, QuantumContractError> {
        Self::build(
            request,
            receipt,
            JobOutcome::Cancelled,
            Vec::new(),
            None,
            completed_at,
        )
    }

    fn build(
        request: &ExecutionRequest,
        receipt: &JobReceipt,
        outcome: JobOutcome,
        mut samples: Vec<ExecutionSample>,
        failure: Option<JobFailure>,
        completed_at: DateTime<Utc>,
    ) -> Result<Self, QuantumContractError> {
        request.validate()?;
        receipt.validate_against(request)?;
        samples.sort_by(compare_samples);
        let mut result = Self {
            schema_version: JOB_RESULT_SCHEMA_VERSION.to_owned(),
            result_sha256: receipt.receipt_sha256.clone(),
            receipt_sha256: receipt.receipt_sha256.clone(),
            request_sha256: request.request_sha256.clone(),
            payload_sha256: request.payload.payload_sha256.clone(),
            backend_id: receipt.backend_id.clone(),
            job_id: receipt.job_id.clone(),
            outcome,
            samples,
            failure,
            native_rescoring: None,
            completed_at,
            limitations: vec![NO_QUANTUM_ADVANTAGE_CLAIM.to_owned()],
        };
        result.refresh_identity()?;
        result.validate_against(request, receipt)?;
        Ok(result)
    }

    fn identity(&self) -> ResultIdentity<'_> {
        ResultIdentity {
            schema_version: &self.schema_version,
            receipt_sha256: &self.receipt_sha256,
            request_sha256: &self.request_sha256,
            payload_sha256: &self.payload_sha256,
            backend_id: &self.backend_id,
            job_id: &self.job_id,
            outcome: self.outcome,
            samples: &self.samples,
            failure: &self.failure,
            native_rescoring: &self.native_rescoring,
            completed_at: &self.completed_at,
            limitations: &self.limitations,
        }
    }

    fn refresh_identity(&mut self) -> Result<(), QuantumContractError> {
        self.result_sha256 = hash_value(&self.identity())?;
        Ok(())
    }

    /// Attach deterministic native rescoring and refresh the result identity.
    pub fn with_native_rescoring(
        mut self,
        request: &ExecutionRequest,
        receipt: &JobReceipt,
        profile: &BenchmarkProfile,
    ) -> Result<Self, QuantumContractError> {
        self.validate_against(request, receipt)?;
        if self.outcome != JobOutcome::Succeeded {
            return Err(QuantumContractError::InvalidContract(
                "native rescoring requires a successful result".to_owned(),
            ));
        }
        self.native_rescoring = Some(NativeRescoringReport::build(
            &request.payload.native_rescoring,
            &self.samples,
            profile,
        )?);
        self.refresh_identity()?;
        self.validate_native_rescoring(request, profile)?;
        Ok(self)
    }

    /// Validate immutable linkage, terminal semantics, samples, and identity.
    pub fn validate_against(
        &self,
        request: &ExecutionRequest,
        receipt: &JobReceipt,
    ) -> Result<(), QuantumContractError> {
        require_schema(
            "job result",
            &self.schema_version,
            JOB_RESULT_SCHEMA_VERSION,
        )?;
        request.validate()?;
        receipt.validate_against(request)?;
        if receipt.state != self.outcome.receipt_state() {
            return Err(QuantumContractError::InvalidContract(format!(
                "result outcome {:?} does not match receipt state {:?}",
                self.outcome, receipt.state
            )));
        }
        if self.receipt_sha256 != receipt.receipt_sha256
            || self.request_sha256 != request.request_sha256
            || self.payload_sha256 != request.payload.payload_sha256
            || self.backend_id != receipt.backend_id
            || self.job_id != receipt.job_id
        {
            return Err(QuantumContractError::InvalidContract(
                "job result is not linked to the supplied receipt and request".to_owned(),
            ));
        }
        match self.outcome {
            JobOutcome::Succeeded if self.samples.is_empty() || self.failure.is_some() => {
                return Err(QuantumContractError::InvalidContract(
                    "successful results require samples and cannot carry a failure".to_owned(),
                ));
            }
            JobOutcome::Failed if !self.samples.is_empty() || self.failure.is_none() => {
                return Err(QuantumContractError::InvalidContract(
                    "failed results require one failure and cannot carry samples".to_owned(),
                ));
            }
            JobOutcome::Cancelled if !self.samples.is_empty() || self.failure.is_some() => {
                return Err(QuantumContractError::InvalidContract(
                    "cancelled results cannot carry samples or a failure".to_owned(),
                ));
            }
            _ => {}
        }
        if let Some(failure) = &self.failure {
            failure.validate()?;
        }
        let mut assignments = BTreeSet::new();
        for sample in &self.samples {
            sample.validate_for(&request.payload.problem)?;
            if !assignments.insert(sample.binary_assignment.as_slice()) {
                return Err(QuantumContractError::InvalidContract(
                    "result contains a duplicate binary assignment".to_owned(),
                ));
            }
        }
        if !self
            .samples
            .windows(2)
            .all(|pair| compare_samples(&pair[0], &pair[1]) != Ordering::Greater)
        {
            return Err(QuantumContractError::InvalidContract(
                "result samples must be sorted by logical energy then assignment".to_owned(),
            ));
        }
        validate_report_lines(&self.limitations, "result limitations")?;
        if !self
            .limitations
            .iter()
            .any(|line| line == NO_QUANTUM_ADVANTAGE_CLAIM)
        {
            return Err(QuantumContractError::InvalidContract(
                "job result is missing its no-advantage interpretation boundary".to_owned(),
            ));
        }
        let computed = hash_value(&self.identity())?;
        if self.result_sha256 != computed {
            return Err(QuantumContractError::IdentityMismatch {
                contract: "job result",
                declared: self.result_sha256.clone(),
                computed,
            });
        }
        Ok(())
    }

    /// Recompute and verify an attached native-rescoring report.
    pub fn validate_native_rescoring(
        &self,
        request: &ExecutionRequest,
        profile: &BenchmarkProfile,
    ) -> Result<(), QuantumContractError> {
        let report = self.native_rescoring.as_ref().ok_or_else(|| {
            QuantumContractError::InvalidContract(
                "job result has no native rescoring report".to_owned(),
            )
        })?;
        report.validate(&request.payload.native_rescoring, &self.samples, profile)
    }
}

fn compare_samples(left: &ExecutionSample, right: &ExecutionSample) -> Ordering {
    left.logical_energy
        .total_cmp(&right.logical_energy)
        .then_with(|| left.binary_assignment.cmp(&right.binary_assignment))
}

/// One versioned response from polling an executor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionUpdate {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Latest content-addressed job receipt.
    pub receipt: JobReceipt,
    /// Terminal result, present exactly when the receipt is terminal.
    pub result: Option<JobResult>,
}

impl ExecutionUpdate {
    /// Construct a non-terminal polling update.
    pub fn pending(
        request: &ExecutionRequest,
        receipt: JobReceipt,
    ) -> Result<Self, QuantumContractError> {
        let update = Self {
            schema_version: EXECUTION_UPDATE_SCHEMA_VERSION.to_owned(),
            receipt,
            result: None,
        };
        update.validate(request)?;
        Ok(update)
    }

    /// Construct a terminal polling update.
    pub fn complete(
        request: &ExecutionRequest,
        receipt: JobReceipt,
        result: JobResult,
    ) -> Result<Self, QuantumContractError> {
        let update = Self {
            schema_version: EXECUTION_UPDATE_SCHEMA_VERSION.to_owned(),
            receipt,
            result: Some(result),
        };
        update.validate(request)?;
        Ok(update)
    }

    /// Validate update state and immutable request linkage.
    pub fn validate(&self, request: &ExecutionRequest) -> Result<(), QuantumContractError> {
        require_schema(
            "execution update",
            &self.schema_version,
            EXECUTION_UPDATE_SCHEMA_VERSION,
        )?;
        self.receipt.validate_against(request)?;
        match (&self.result, self.receipt.state.is_terminal()) {
            (None, false) => Ok(()),
            (Some(result), true) => result.validate_against(request, &self.receipt),
            (None, true) => Err(QuantumContractError::InvalidContract(
                "terminal execution update requires a job result".to_owned(),
            )),
            (Some(_), false) => Err(QuantumContractError::InvalidContract(
                "non-terminal execution update cannot carry a job result".to_owned(),
            )),
        }
    }
}

fn require_nonempty(label: &str, value: &str) -> Result<(), QuantumContractError> {
    if value.trim().is_empty() {
        Err(QuantumContractError::InvalidContract(format!(
            "{label} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn require_safe_text(label: &str, value: &str) -> Result<(), QuantumContractError> {
    require_nonempty(label, value)?;
    if value.len() > 512 || value.chars().any(char::is_control) {
        return Err(QuantumContractError::InvalidContract(format!(
            "{label} must be at most 512 characters and contain no control characters"
        )));
    }
    Ok(())
}

fn validate_finite(
    values: impl IntoIterator<Item = f64>,
    label: &str,
) -> Result<(), QuantumContractError> {
    if values.into_iter().any(|value| !value.is_finite()) {
        Err(QuantumContractError::InvalidContract(format!(
            "{label} must be finite"
        )))
    } else {
        Ok(())
    }
}

fn validate_report_lines(lines: &[String], label: &str) -> Result<(), QuantumContractError> {
    for line in lines {
        require_safe_text(label, line)?;
    }
    Ok(())
}
