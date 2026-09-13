use qbm_domain::{
    Approval, AuditFinding, AuditSummary, Decision, IntakeSourceKind, InventoryEntry, ProjectId,
    RunEvent, RunId, RunMode, RunState, ScanDiagnostic, Sha256Digest, SourceAcquisition,
    SourceAcquisitionKind, StageOutput,
};
use serde::{Deserialize, Serialize};

pub(crate) const WORKFLOW_STAGES: [(&str, &str, &str); 14] = [
    (
        "inventory",
        "Check files",
        "Review what was included, excluded, or blocked before evidence is frozen.",
    ),
    (
        "source-snapshot",
        "Freeze evidence",
        "Confirm the immutable file-to-content snapshot used by every later stage.",
    ),
    (
        "raw-graph",
        "Map structure",
        "Review the domain-blind structural map and parser diagnostics.",
    ),
    (
        "generic-audit",
        "Generic source audit",
        "Review source-level findings and explicit coverage limits before semantic interpretation.",
    ),
    (
        "adapter-detection",
        "Detect semantic adapters",
        "Match installed, data-only biomedical adapters against the approved source graph.",
    ),
    (
        "adapter-plan",
        "Lock adapter plan",
        "Approve the exact eligible adapter packages selected for semantic projection.",
    ),
    (
        "semantic-graph",
        "Project biomedical semantics",
        "Map source evidence into adapter-defined biomedical concepts and relationships.",
    ),
    (
        "semantic-audit",
        "Audit semantic model",
        "Evaluate evidence-linked policies declared by the selected biomedical adapters.",
    ),
    (
        "projection-catalog",
        "Resolve semantic views",
        "Confirm which adapter-defined views are available for benchmark profiling.",
    ),
    (
        "biomedical-profile",
        "Approve qbm.profile",
        "Review the normalized biomedical inputs, outcomes, relationships, costs, weights and constraints before optimization.",
    ),
    (
        "profile-comparison",
        "Compare profile versions",
        "Measure input, outcome, relationship, reachability, coverage, panel-size and QUBO changes against a compatible baseline.",
    ),
    (
        "classical-optimization",
        "Run classical baselines",
        "Measure coverage at K and minimum panels with deterministic exact or heuristic solvers.",
    ),
    (
        "qubo-ising-validation",
        "Validate QUBO and Ising",
        "Build the approved formulation, verify its energy conversion and assess optional quantum execution readiness.",
    ),
    (
        "full-report",
        "Review full report",
        "Approve the complete audit, semantic, comparison, optimization and reproducibility record.",
    ),
];

#[derive(Debug, Serialize)]
pub(crate) struct BootstrapView {
    pub schema_version: &'static str,
    pub csrf_token: String,
    pub service: &'static str,
    pub version: &'static str,
    pub source_types: [&'static str; 3],
    pub max_upload_bytes: u64,
    pub recent_runs: Vec<RunListItem>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RunListItem {
    pub run_id: RunId,
    pub project_id: ProjectId,
    pub project_name: String,
    pub source_kind: Option<SourceAcquisitionKind>,
    pub state: RunState,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectView {
    pub id: ProjectId,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SourceView {
    pub id: Sha256Digest,
    pub kind: SourceAcquisitionKind,
    pub source_locator: String,
    pub repository_id: Option<u64>,
    pub requested_revision: Option<String>,
    pub resolved_revision: Option<String>,
    pub provider_api_version: Option<String>,
    pub content_sha256: Sha256Digest,
    pub total_bytes: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<&SourceAcquisition> for SourceView {
    fn from(value: &SourceAcquisition) -> Self {
        Self {
            id: value.id.clone(),
            kind: value.kind,
            source_locator: value.source_locator.clone(),
            repository_id: value.repository_id,
            requested_revision: value.requested_revision.clone(),
            resolved_revision: value.resolved_revision.clone(),
            provider_api_version: value.provider_api_version.clone(),
            content_sha256: value.content_sha256.clone(),
            total_bytes: value.total_bytes,
            created_at: value.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct StageView {
    pub id: &'static str,
    pub label: &'static str,
    pub purpose: &'static str,
    pub status: String,
    pub output: Option<StageOutput>,
}

#[derive(Debug, Serialize)]
pub(crate) struct InventoryView {
    pub id: qbm_domain::InventoryId,
    pub inventory_hash: Sha256Digest,
    pub source_kind: IntakeSourceKind,
    pub source_container_hash: Option<Sha256Digest>,
    pub included_files: u64,
    pub excluded_entries: u64,
    pub blocked_entries: u64,
    pub total_included_bytes: u64,
    pub exceptions: Vec<InventoryEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SnapshotView {
    pub id: Sha256Digest,
    pub policy_hash: Sha256Digest,
    pub files: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct GraphView {
    pub id: Sha256Digest,
    pub policy_hash: Sha256Digest,
    pub nodes: u64,
    pub edges: u64,
    pub diagnostics: Vec<ScanDiagnostic>,
    pub parser_coverage: ParserCoverageView,
    pub language_scan: LanguageScanView,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LanguageScanView {
    pub schema_version: &'static str,
    pub rust_modules: u64,
    pub rust_items: u64,
    pub rust_imports: u64,
    pub python_modules: u64,
    pub python_classes: u64,
    pub python_functions: u64,
    pub python_tests: u64,
    pub python_calls: u64,
    pub python_imports: u64,
    pub dependency_count: u64,
    pub dependency_samples: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct FormatCoverageView {
    pub format: String,
    pub files: u64,
    pub bytes: u64,
    pub parsed_files: u64,
    pub parsed_bytes: u64,
    pub status: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ParserCoverageView {
    pub schema_version: &'static str,
    pub total_files: u64,
    pub total_bytes: u64,
    pub parser_eligible_files: u64,
    pub parser_eligible_bytes: u64,
    pub parsed_files: u64,
    pub parsed_bytes: u64,
    pub file_coverage_basis_points: u16,
    pub byte_coverage_basis_points: u16,
    pub formats: Vec<FormatCoverageView>,
    pub unsupported_files: u64,
    pub unsupported_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CheckResultView {
    pub check_id: &'static str,
    pub family: &'static str,
    pub status: &'static str,
    pub findings: u64,
    pub explanation: String,
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AuditCoverageView {
    pub schema_version: &'static str,
    pub audit_scope: &'static str,
    pub completion_level: &'static str,
    pub completion_message: &'static str,
    pub checks: Vec<CheckResultView>,
    pub limitations: Vec<String>,
    pub next_required_stage: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReportView {
    pub id: Sha256Digest,
    pub summary: AuditSummary,
    pub findings: Vec<AuditFinding>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkflowView {
    pub schema_version: &'static str,
    pub project: ProjectView,
    pub run_id: RunId,
    pub run_mode: RunMode,
    pub run_state: RunState,
    pub source: SourceView,
    pub stages: Vec<StageView>,
    pub inventory: Option<InventoryView>,
    pub snapshot: Option<SnapshotView>,
    pub graph: Option<GraphView>,
    pub report: Option<ReportView>,
    pub audit_coverage: Option<AuditCoverageView>,
    pub adapter_detection: Option<serde_json::Value>,
    pub adapter_plan: Option<serde_json::Value>,
    pub semantic_graph: Option<serde_json::Value>,
    pub semantic_report: Option<serde_json::Value>,
    pub projection_catalog: Option<serde_json::Value>,
    pub biomedical_profile: Option<serde_json::Value>,
    pub profile_comparison: Option<serde_json::Value>,
    pub classical_optimization: Option<serde_json::Value>,
    pub qubo_ising_validation: Option<serde_json::Value>,
    pub full_report: Option<serde_json::Value>,
    pub event_chain_valid: bool,
    pub report_download_url: Option<String>,
    pub profile_download_url: Option<String>,
    pub optimization_download_url: Option<String>,
    pub wiring_diagnostic_download_url: Option<String>,
    pub quantum_export_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GithubImportRequest {
    pub repository_url: String,
    pub reference: Option<String>,
    pub project_name: Option<String>,
    /// Absent means governed, so an older client keeps its review gates.
    pub run_mode: Option<BrowserRunMode>,
}

/// The two run modes a browser may ask for.
///
/// The platform has other modes; they are internal audit settings, and a
/// browser naming one is rejected rather than quietly mapped onto one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrowserRunMode {
    /// Review the exact bytes at every material checkpoint.
    Governed,
    /// Record and hash every stage, accepting each by policy.
    Express,
}

impl From<BrowserRunMode> for RunMode {
    fn from(value: BrowserRunMode) -> Self {
        match value {
            BrowserRunMode::Governed => Self::Governed,
            BrowserRunMode::Express => Self::Express,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DecisionRequest {
    pub stage_id: String,
    pub expected_output_hash: String,
    pub decision: Decision,
    pub actor_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ReportBundle {
    pub schema_version: &'static str,
    pub project: ProjectView,
    pub source: SourceView,
    pub run_id: RunId,
    pub run_state: RunState,
    pub approvals: Vec<Approval>,
    pub inventory: InventoryView,
    pub snapshot: SnapshotView,
    pub graph: GraphView,
    pub generic_report: ReportView,
    pub audit_coverage: AuditCoverageView,
    pub full_report: Option<serde_json::Value>,
    pub events: Vec<RunEvent>,
    pub event_chain_valid: bool,
}
