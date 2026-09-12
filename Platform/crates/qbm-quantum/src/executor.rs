//! Pluggable executor interface with no provider implementation or configuration.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    BackendCapability, EXECUTOR_ERROR_SCHEMA_VERSION, ExecutionReadiness, ExecutionRequest,
    ExecutionUpdate, JobReceipt,
};

/// Stable category for a sanitized executor-boundary failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorErrorKind {
    /// Backend execution is intentionally unavailable.
    NotConfigured,
    /// The request exceeds a declared backend capability.
    Unsupported,
    /// The provider did not accept the request.
    Submission,
    /// A submitted job could not be queried.
    Polling,
    /// A provider response could not be converted to the neutral contract.
    InvalidResponse,
    /// A transient provider-side failure may be retried.
    Unavailable,
}

/// Sanitized error returned at the pluggable executor boundary.
///
/// Implementations must not copy credentials, authorization headers, request
/// bodies, or unreviewed provider responses into `code` or `summary`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(deny_unknown_fields)]
#[error("{code}: {summary}")]
pub struct ExecutorError {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Stable broad category.
    pub kind: ExecutorErrorKind,
    /// Executor-defined stable machine code.
    pub code: String,
    /// Safe human-readable summary.
    pub summary: String,
    /// Whether retrying the exact immutable operation may succeed.
    pub retryable: bool,
}

impl ExecutorError {
    /// Construct a sanitized executor error.
    #[must_use]
    pub fn new(
        kind: ExecutorErrorKind,
        code: impl Into<String>,
        summary: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            schema_version: EXECUTOR_ERROR_SCHEMA_VERSION.to_owned(),
            kind,
            code: code.into(),
            summary: summary.into(),
            retryable,
        }
    }

    /// Standard error for an absent optional executor.
    #[must_use]
    pub fn not_configured() -> Self {
        Self::new(
            ExecutorErrorKind::NotConfigured,
            "not_configured",
            "quantum execution is not configured; approved payload export remains available",
            false,
        )
    }
}

/// Provider adapter implemented outside this pure contract crate.
///
/// Implementations own provider clients, endpoints, and secrets. Those values
/// are deliberately absent from every argument and return type here.
pub trait QuantumExecutor: Send + Sync {
    /// Return public, non-secret capabilities for request preflight checks.
    fn capability(&self) -> &BackendCapability;

    /// Return a browser-safe readiness report.
    fn readiness(&self) -> ExecutionReadiness {
        ExecutionReadiness::ready(self.capability().clone())
    }

    /// Submit one validated immutable request.
    fn submit(&self, request: &ExecutionRequest) -> Result<JobReceipt, ExecutorError>;

    /// Poll a previously submitted receipt.
    fn poll(
        &self,
        request: &ExecutionRequest,
        receipt: &JobReceipt,
    ) -> Result<ExecutionUpdate, ExecutorError>;
}
