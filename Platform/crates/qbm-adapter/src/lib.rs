//! Safe, deterministic interpreter for declarative Q-BenchMed adapter packages.
//!
//! Adapter packages are data. This crate never loads native libraries, executes
//! package code, starts processes, or performs network access. Every operation
//! is bounded and derives its inputs from immutable domain objects.

mod engine;
mod strict_json;
mod validation;

use qbm_canonical::CanonicalError;
use qbm_store::StoreError;
use thiserror::Error;

pub use engine::AdapterEngine;

/// Resource ceilings enforced by the declarative interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterLimits {
    /// Largest accepted adapter-package JSON document.
    pub max_package_bytes: usize,
    /// Maximum number of detection rules in one package.
    pub max_detection_rules: usize,
    /// Maximum matching raw-node identities retained as evidence per detection rule.
    pub max_detection_evidence_samples: usize,
    /// Maximum installed packages considered by one detection operation.
    pub max_registry_packages: usize,
    /// Maximum canonical bytes for all candidates in one detection report.
    pub max_total_detection_output_bytes: usize,
    /// Maximum exact adapter packages selected in one plan.
    pub max_selected_adapters: usize,
    /// Maximum total node and edge mapping rules in one package.
    pub max_mapping_rules: usize,
    /// Maximum attributes declared by one semantic node mapping.
    pub max_attributes_per_rule: usize,
    /// Maximum canonical bytes in one literal or emitted attribute map.
    pub max_attribute_bytes: usize,
    /// Maximum semantic attribute bytes emitted by one projection operation.
    pub max_total_attribute_bytes: usize,
    /// Maximum canonical bytes for all emitted semantic nodes and edges.
    pub max_total_semantic_output_bytes: usize,
    /// Maximum number of semantic policy rules in one package.
    pub max_policy_rules: usize,
    /// Maximum number of projections in one package.
    pub max_projections: usize,
    /// Maximum number of conformance fixtures in one package.
    pub max_fixtures: usize,
    /// Maximum raw nodes across one package's fixtures.
    pub max_fixture_nodes: usize,
    /// Maximum raw edges across one package's fixtures.
    pub max_fixture_edges: usize,
    /// Maximum semantic nodes emitted by one projection operation.
    pub max_semantic_nodes: usize,
    /// Maximum semantic edges emitted by one projection operation.
    pub max_semantic_edges: usize,
    /// Maximum findings emitted by one semantic audit.
    pub max_findings: usize,
    /// Maximum semantic node identities retained in one finding.
    pub max_finding_affected_nodes: usize,
    /// Maximum evidence locations retained in one finding.
    pub max_finding_evidence_locations: usize,
    /// Maximum canonical bytes for all findings emitted by one audit.
    pub max_total_audit_output_bytes: usize,
    /// Maximum node and edge memberships across one projection catalog.
    pub max_total_projection_members: usize,
    /// Maximum canonical bytes for all resolved projections in one catalog.
    pub max_total_projection_output_bytes: usize,
    /// Maximum artifact bytes read to resolve one evidence value.
    pub max_evidence_bytes: u64,
    /// Maximum artifact bytes cumulatively parsed during one projection.
    pub max_total_evidence_bytes: u64,
    /// Maximum nested array/object depth in a resolved evidence value.
    pub max_value_depth: usize,
    /// Maximum array/object/scalar elements in a resolved evidence value.
    pub max_value_elements: usize,
    /// Maximum deterministic rule/node/edge evaluations in one operation.
    pub max_rule_evaluations: u64,
}

impl Default for AdapterLimits {
    fn default() -> Self {
        Self {
            max_package_bytes: 2 * 1024 * 1024,
            max_detection_rules: 256,
            max_detection_evidence_samples: 16,
            max_registry_packages: 256,
            max_total_detection_output_bytes: 64 * 1024 * 1024,
            max_selected_adapters: 16,
            max_mapping_rules: 1_024,
            max_attributes_per_rule: 64,
            max_attribute_bytes: 256 * 1024,
            max_total_attribute_bytes: 64 * 1024 * 1024,
            max_total_semantic_output_bytes: 128 * 1024 * 1024,
            max_policy_rules: 1_024,
            max_projections: 256,
            max_fixtures: 64,
            max_fixture_nodes: 16_384,
            max_fixture_edges: 32_768,
            max_semantic_nodes: 250_000,
            max_semantic_edges: 500_000,
            max_findings: 250_000,
            max_finding_affected_nodes: 4_096,
            max_finding_evidence_locations: 4_096,
            max_total_audit_output_bytes: 64 * 1024 * 1024,
            max_total_projection_members: 500_000,
            max_total_projection_output_bytes: 128 * 1024 * 1024,
            max_evidence_bytes: 2 * 1024 * 1024,
            max_total_evidence_bytes: 64 * 1024 * 1024,
            max_value_depth: 64,
            max_value_elements: 250_000,
            max_rule_evaluations: 50_000_000,
        }
    }
}

/// Failure raised by the declarative adapter interpreter.
#[derive(Debug, Error)]
pub enum AdapterError {
    /// Package JSON was invalid, non-canonicalizable, or contained duplicate keys.
    #[error("adapter package JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// A canonical content identity could not be generated.
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    /// Immutable artifact storage failed validation or I/O.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A hard interpreter resource limit was exceeded.
    #[error("adapter {kind} exceeds the configured limit of {maximum}")]
    ResourceLimit {
        /// Bounded resource category.
        kind: &'static str,
        /// Configured ceiling.
        maximum: u64,
    },
    /// Immutable predecessor objects did not form a valid chain.
    #[error("invalid adapter relationship: {0}")]
    InvalidRelationship(String),
    /// A requested installed adapter package was missing or inconsistent.
    #[error("adapter package {0} is not present in the supplied registry snapshot")]
    MissingPackage(String),
    /// Evidence could not be resolved inside the exact approved snapshot.
    #[error("adapter evidence cannot be resolved safely: {0}")]
    Evidence(String),
    /// The package failed static or fixture conformance and cannot execute.
    #[error("adapter package {0} has not passed conformance")]
    NonConformant(String),
}
