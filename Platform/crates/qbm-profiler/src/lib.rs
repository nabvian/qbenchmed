//! Evidence-backed projection into Q-BenchMed benchmark profiles.
//!
//! Callers provide already-parsed JSON or YAML values together with immutable
//! artifact identities and normalized relative paths. This crate performs no
//! filesystem or network I/O, never executes source or package code, and does
//! not infer biomedical meaning from arbitrary source-language identifiers.
//! Projection is limited to an explicit profile document or a deliberately
//! small structured biomedical heuristic, and every result requires review.

mod conditions;
mod project;
mod validation;

use qbm_benchmark::{BenchmarkError, BenchmarkProfile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub use project::Profiler;

/// Schema identifier serialized with every projection candidate.
pub const PROJECTION_CANDIDATE_SCHEMA_VERSION: &str = "qbm.projection-candidate/v1";

/// One bounded, pre-parsed JSON or YAML document.
///
/// YAML callers should deserialize into [`serde_json::Value`] before calling
/// the profiler. Keeping parsing outside this crate makes the trust boundary
/// explicit and prevents this component from reading or executing source data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDocument {
    /// Normalized, non-empty relative path used in evidence records.
    pub path: String,
    /// Immutable portable artifact identity (for example a SHA-256 digest).
    pub artifact_id: String,
    /// Already-parsed JSON-compatible document value.
    pub value: Value,
}

/// Stable caller-owned metadata for heuristic projection across source versions.
///
/// Supplying `profile_id` is recommended whenever candidates will be compared
/// across uploads. Context values are not presented as document extraction;
/// they are retained as explicit review assumptions with source evidence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectionContext {
    /// Stable logical profile identity, independent of artifact content hashes.
    pub profile_id: Option<String>,
    /// Caller-owned human-readable title.
    pub title: Option<String>,
    /// Caller-owned biomedical area.
    pub biomedical_area: Option<String>,
    /// Caller-owned population, cohort, or indication boundary.
    pub population: Option<String>,
    /// Immutable source revision when the caller has one.
    pub source_revision: Option<String>,
    /// Integration identity recorded in benchmark provenance.
    pub generated_by: Option<String>,
}

impl SourceDocument {
    /// Construct a pre-parsed source document.
    #[must_use]
    pub fn new(path: impl Into<String>, artifact_id: impl Into<String>, value: Value) -> Self {
        Self {
            path: path.into(),
            artifact_id: artifact_id.into(),
            value,
        }
    }
}

/// Host-controlled ceilings for document traversal and projected output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfilerLimits {
    /// Maximum number of source documents in one projection request.
    pub max_documents: usize,
    /// Maximum nesting depth in any pre-parsed value.
    pub max_value_depth: usize,
    /// Maximum cumulative values and object keys inspected.
    pub max_value_elements: usize,
    /// Maximum cumulative UTF-8 bytes in keys and string values.
    pub max_total_string_bytes: usize,
    /// Maximum number of structured rules inspected.
    pub max_rules: usize,
    /// Maximum distinct projected inputs.
    pub max_inputs: usize,
    /// Maximum distinct projected outcomes.
    pub max_outcomes: usize,
    /// Maximum distinct incidence relationships.
    pub max_relationships: usize,
    /// Maximum evidence records in the returned candidate.
    pub max_evidence: usize,
}

impl Default for ProfilerLimits {
    fn default() -> Self {
        Self {
            max_documents: 128,
            max_value_depth: 64,
            // A real knowledge bundle is hundreds of files deep. At 250_000
            // the profiler refused four of the projects it was pointed at
            // purely on size, having understood them perfectly well. The bound
            // exists to stop unbounded work, not to cap a project's ambition.
            max_value_elements: 4_000_000,
            max_total_string_bytes: 16 * 1024 * 1024,
            max_rules: 8_192,
            max_inputs: 16_384,
            max_outcomes: 16_384,
            max_relationships: 100_000,
            max_evidence: 250_000,
        }
    }
}

/// Projection strategy that produced a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionMethod {
    /// A canonical document was decoded directly as a benchmark profile.
    ExplicitProfile,
    /// Exact structured biomedical keys were projected by the built-in heuristic.
    BiomedicalHeuristic,
}

/// Review state of a projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionStatus {
    /// A human must review and approve the candidate before optimization.
    RequiresApproval,
}

/// Severity of a non-fatal projection diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    /// Context useful during review.
    Info,
    /// Ambiguity or ignored data that deserves reviewer attention.
    Warning,
}

/// Immutable evidence linking one target field to one source location.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionEvidence {
    /// Source artifact identity supplied by the caller.
    pub artifact_id: String,
    /// Normalized source path supplied by the caller.
    pub path: String,
    /// RFC 6901 JSON Pointer into the pre-parsed source value.
    pub pointer: String,
    /// Stable field path in the projected candidate.
    pub target_field: String,
}

/// One deterministic, evidence-linked observation made during projection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionDiagnostic {
    /// Stable machine-readable diagnostic code.
    pub code: String,
    /// Review severity.
    pub severity: DiagnosticSeverity,
    /// Human-readable explanation.
    pub message: String,
    /// Sorted source evidence for the observation.
    pub evidence: Vec<ProjectionEvidence>,
}

/// One value introduced by the profiler rather than extracted from a document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionAssumption {
    /// Stable machine-readable assumption code.
    pub code: String,
    /// Stable field path receiving the default.
    pub target_field: String,
    /// Deterministic textual representation of the introduced value.
    pub value: String,
    /// Human-readable rationale requiring reviewer confirmation.
    pub message: String,
    /// Sorted source evidence that caused the field to be needed.
    pub evidence: Vec<ProjectionEvidence>,
}

/// A validated benchmark profile plus its review record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionCandidate {
    /// Candidate envelope schema.
    pub schema_version: String,
    /// Projection strategy used.
    pub method: ProjectionMethod,
    /// Human-review state. Projection never grants automatic approval.
    pub status: ProjectionStatus,
    /// Confidence in faithful extraction, in basis points from 0 through 10,000.
    pub confidence_bps: u16,
    /// Validated optimizer input coordinated with `qbm-benchmark`.
    pub profile: BenchmarkProfile,
    /// Strictly sorted unique fields read from source documents.
    pub extracted_fields: Vec<String>,
    /// Strictly sorted unique fields introduced by the profiler.
    pub defaulted_fields: Vec<String>,
    /// Strictly sorted unique source-to-target evidence records.
    pub evidence: Vec<ProjectionEvidence>,
    /// Strictly sorted unique non-fatal diagnostics.
    pub diagnostics: Vec<ProjectionDiagnostic>,
    /// Strictly sorted unique defaults and interpretation assumptions.
    pub assumptions: Vec<ProjectionAssumption>,
}

impl ProjectionCandidate {
    /// Validate the benchmark profile and projection-envelope invariants.
    pub fn validate(&self) -> Result<(), ProfilerError> {
        validation::validate_candidate(self)
    }
}

/// Errors returned before a reviewable, valid candidate can be produced.
#[derive(Debug, Error)]
pub enum ProfilerError {
    /// No document was supplied.
    #[error("at least one pre-parsed source document is required")]
    EmptyInput,
    /// Source metadata or normalized structure is invalid.
    #[error("invalid source document {path:?}: {message}")]
    InvalidDocument {
        /// Supplied document path.
        path: String,
        /// Invariant violation.
        message: String,
    },
    /// Caller-owned stable projection context is invalid.
    #[error("invalid projection context field {field}: {message}")]
    InvalidContext {
        /// Context field name.
        field: &'static str,
        /// Invariant violation.
        message: String,
    },
    /// A configured resource ceiling was exceeded.
    #[error("profiler {kind} exceeds the configured maximum of {maximum}")]
    ResourceLimit {
        /// Bounded resource category.
        kind: &'static str,
        /// Configured ceiling.
        maximum: usize,
    },
    /// More than one explicit profile document made selection ambiguous.
    #[error("multiple explicit profile documents are ambiguous: {0:?}")]
    AmbiguousExplicitProfiles(Vec<String>),
    /// An explicit document could not be decoded as the benchmark schema.
    #[error("explicit profile document {path:?} is invalid: {message}")]
    InvalidExplicitProfile {
        /// Explicit document path.
        path: String,
        /// Decode or envelope failure.
        message: String,
    },
    /// The projected profile failed `qbm-benchmark` validation.
    #[error("projected benchmark profile is invalid: {0}")]
    InvalidProfile(#[source] BenchmarkError),
    /// Structured documents did not contain enough biomedical evidence.
    #[error("no conservative biomedical profile projection is available: {0}")]
    NoProjection(String),
    /// The projection envelope itself is inconsistent.
    #[error("invalid projection candidate: {0}")]
    InvalidCandidate(String),
}

/// Project bounded documents with the default conservative limits.
pub fn project_documents(
    documents: &[SourceDocument],
) -> Result<ProjectionCandidate, ProfilerError> {
    Profiler::new().project(documents)
}

/// Project bounded documents with stable caller-owned metadata.
pub fn project_documents_with_context(
    documents: &[SourceDocument],
    context: &ProjectionContext,
) -> Result<ProjectionCandidate, ProfilerError> {
    Profiler::new().project_with_context(documents, context)
}
