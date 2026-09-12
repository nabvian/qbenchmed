//! Non-executing, scanner-neutral project graph and generic audit policies.

mod builder;
mod policies;
mod python;

use qbm_canonical::CanonicalError;
use qbm_domain::{AuditReport, GraphPolicy, RawProjectGraph, SourceSnapshot};
use qbm_store::{PlatformStore, StoreError};
use thiserror::Error;

/// Generic graph and audit failures.
#[derive(Debug, Error)]
pub enum AuditError {
    /// Artifact persistence or integrity verification failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Canonical identity generation failed.
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    /// JSON conversion failed.
    #[error("structured source conversion failed: {0}")]
    Json(#[from] serde_json::Error),
    /// A graph resource bound was reached; a partial graph is never returned.
    #[error("graph exceeds configured {kind} limit of {maximum}")]
    GraphLimit {
        /// Resource category.
        kind: &'static str,
        /// Configured maximum.
        maximum: u64,
    },
}

/// Generic audit engine reading only immutable content-addressed artifacts.
#[derive(Debug, Clone)]
pub struct AuditEngine {
    store: PlatformStore,
}

impl AuditEngine {
    /// Create an engine using an existing platform store.
    #[must_use]
    pub fn new(store: PlatformStore) -> Self {
        Self { store }
    }

    /// Build a deterministic raw graph without compiling or executing source code.
    pub fn build_graph(
        &self,
        snapshot: &SourceSnapshot,
        policy: &GraphPolicy,
    ) -> Result<RawProjectGraph, AuditError> {
        builder::build_graph(&self.store, snapshot, policy)
    }

    /// Apply scanner-neutral structural policies to an approved raw graph.
    pub fn audit(&self, graph: &RawProjectGraph) -> Result<AuditReport, AuditError> {
        policies::audit(graph)
    }
}
