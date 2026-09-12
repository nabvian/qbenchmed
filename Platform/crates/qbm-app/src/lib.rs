//! Application use cases shared by CLI, HTTP API and future browser UI.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use chrono::Utc;
use qbm_adapter::{AdapterEngine, AdapterError};
use qbm_audit::{AuditEngine, AuditError};
use qbm_domain::{
    ActorId, AdapterConformanceReport, AdapterDetectionReport, AdapterPackage, AdapterPlan,
    AdapterRecord, Approval, ApprovalRequest, ArtifactRecord, AuditReport, Decision, GraphPolicy,
    IntakePolicy, Inventory, InventoryId, Project, ProjectId, ProjectionCatalog, RawProjectGraph,
    Run, RunEvent, RunId, RunMode, SemanticAuditReport, SemanticGraph, Sha256Digest,
    SourceAcquisition, SourceSnapshot, StageId, StageOutput,
};
use qbm_intake::{IntakeEngine, IntakeError};
use qbm_store::{PlatformStore, StoreError};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

/// Application-layer errors.
#[derive(Debug, Error)]
pub enum AppError {
    /// Domain validation failed.
    #[error(transparent)]
    Domain(#[from] qbm_domain::DomainError),
    /// Persistence or integrity operation failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Safe intake or snapshot construction failed.
    #[error(transparent)]
    Intake(#[from] IntakeError),
    /// Generic graph construction or audit failed.
    #[error(transparent)]
    Audit(#[from] AuditError),
    /// Declarative adapter validation or interpretation failed.
    #[error(transparent)]
    Adapter(#[from] AdapterError),
    /// Filesystem inspection failed.
    #[error("cannot inspect source path: {0}")]
    Io(#[from] std::io::Error),
    /// A project source must be an existing directory or file.
    #[error("project source {0} does not exist or is not a file/directory")]
    InvalidSource(PathBuf),
    /// A local artifact source must be a regular file.
    #[error("artifact source {0} is not an existing regular file")]
    InvalidArtifact(PathBuf),
    /// Adapter package files are intentionally bounded before they are read into memory.
    #[error("adapter package {path} is {size} bytes; maximum is {maximum} bytes")]
    AdapterPackageTooLarge {
        /// Rejected local source.
        path: PathBuf,
        /// Observed file size.
        size: u64,
        /// Application intake ceiling.
        maximum: u64,
    },
    /// A declarative adapter package did not pass its bundled conformance checks.
    #[error("adapter package failed conformance report {report_id}")]
    AdapterConformanceFailed {
        /// Deterministic report identity that can be compared across installations.
        report_id: Sha256Digest,
    },
    /// An inventory belongs to a different run.
    #[error("inventory {inventory_id} belongs to run {actual_run}, not {expected_run}")]
    InventoryRunMismatch {
        /// Inventory record.
        inventory_id: InventoryId,
        /// Run requested by the caller.
        expected_run: RunId,
        /// Run that created the inventory.
        actual_run: RunId,
    },
    /// Built-in stage outputs must be produced by their typed, lineage-checking use case.
    #[error(
        "stage {0:?} is managed by the platform and cannot be completed with an arbitrary hash"
    )]
    ReservedStage(String),
    /// Only explicitly declared derived stages may terminate a run.
    #[error("stage {0:?} is not a supported derived terminal stage")]
    InvalidDerivedTerminalStage(String),
}

const RESERVED_STAGE_IDS: &[&str] = &[
    "inventory",
    "source-snapshot",
    "raw-graph",
    "generic-audit",
    "adapter-detection",
    "adapter-plan",
    "semantic-graph",
    "semantic-audit",
    "projection-catalog",
];
const DERIVED_TERMINAL_STAGE_IDS: &[&str] = &["full-report"];

/// Cohesive platform application service.
#[derive(Debug, Clone)]
pub struct PlatformApp {
    store: PlatformStore,
}

struct PreparedAdapter {
    bytes: Vec<u8>,
    package: AdapterPackage,
    package_hash: Sha256Digest,
    conformance: AdapterConformanceReport,
}

impl PlatformApp {
    /// Open or initialize an application data directory.
    pub fn open(data_directory: impl AsRef<Path>) -> Result<Self, AppError> {
        Ok(Self {
            store: PlatformStore::open(data_directory)?,
        })
    }

    /// Local platform data directory.
    #[must_use]
    pub fn data_directory(&self) -> &Path {
        self.store.root()
    }

    /// Register an existing source directory or supported archive without modifying it.
    pub fn register_project(
        &self,
        id: &str,
        display_name: &str,
        source_path: &Path,
    ) -> Result<Project, AppError> {
        if !source_path.is_dir() && !source_path.is_file() {
            return Err(AppError::InvalidSource(source_path.to_path_buf()));
        }
        let canonical_source = source_path.canonicalize()?;
        Ok(self
            .store
            .create_project(ProjectId::new(id)?, display_name.trim(), canonical_source)?)
    }

    /// Load one project.
    pub fn project(&self, id: &str) -> Result<Project, AppError> {
        Ok(self.store.get_project(&ProjectId::new(id)?)?)
    }

    /// List registered projects.
    pub fn projects(&self) -> Result<Vec<Project>, AppError> {
        Ok(self.store.list_projects()?)
    }

    /// Persist provenance for a project copied into platform-managed storage.
    pub fn calculate_source_acquisition_id(
        &self,
        acquisition: &SourceAcquisition,
    ) -> Result<Sha256Digest, AppError> {
        Ok(PlatformStore::calculate_source_acquisition_id(acquisition)?)
    }

    /// Persist provenance for a project copied into platform-managed storage.
    pub fn save_source_acquisition(
        &self,
        acquisition: &SourceAcquisition,
    ) -> Result<SourceAcquisition, AppError> {
        Ok(self.store.save_source_acquisition(acquisition)?)
    }

    /// Load one immutable managed-source provenance record.
    pub fn source_acquisition(&self, acquisition_id: &str) -> Result<SourceAcquisition, AppError> {
        Ok(self
            .store
            .get_source_acquisition(&Sha256Digest::new(acquisition_id)?)?)
    }

    /// List managed-source provenance, optionally restricted to one project.
    pub fn source_acquisitions(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<SourceAcquisition>, AppError> {
        let project_id = project_id.map(ProjectId::new).transpose()?;
        Ok(self.store.list_source_acquisitions(project_id.as_ref())?)
    }

    /// Pin a run to the exact source version it audits.
    pub fn bind_run_acquisition(
        &self,
        run_id: RunId,
        acquisition_id: &str,
    ) -> Result<SourceAcquisition, AppError> {
        Ok(self
            .store
            .bind_run_acquisition(run_id, &Sha256Digest::new(acquisition_id)?)?)
    }

    /// Load the exact immutable source acquisition pinned to a run.
    pub fn run_acquisition(&self, run_id: RunId) -> Result<SourceAcquisition, AppError> {
        Ok(self.store.get_run_acquisition(run_id)?)
    }

    /// Start a project run.
    pub fn start_run(&self, project_id: &str, mode: RunMode) -> Result<Run, AppError> {
        Ok(self.store.create_run(ProjectId::new(project_id)?, mode)?)
    }

    /// Load one run.
    pub fn run(&self, run_id: RunId) -> Result<Run, AppError> {
        Ok(self.store.get_run(run_id)?)
    }

    /// List runs, optionally for one project.
    pub fn runs(&self, project_id: Option<&str>) -> Result<Vec<Run>, AppError> {
        let project_id = project_id.map(ProjectId::new).transpose()?;
        Ok(self.store.list_runs(project_id.as_ref())?)
    }

    /// Record the current output hash of a material stage.
    pub fn complete_stage(
        &self,
        run_id: RunId,
        stage_id: &str,
        output_hash: &str,
    ) -> Result<StageOutput, AppError> {
        if RESERVED_STAGE_IDS.contains(&stage_id) {
            return Err(AppError::ReservedStage(stage_id.to_owned()));
        }
        Ok(self.store.record_stage_output(
            run_id,
            StageId::new(stage_id)?,
            Sha256Digest::new(output_hash)?,
        )?)
    }

    /// Record an approval, rejection, or change request for an exact output.
    pub fn decide_stage(
        &self,
        run_id: RunId,
        stage_id: &str,
        expected_hash: &str,
        decision: Decision,
        actor_id: &str,
        reason: Option<String>,
    ) -> Result<Approval, AppError> {
        Ok(self.store.decide_stage(ApprovalRequest {
            run_id,
            stage_id: StageId::new(stage_id)?,
            expected_output_hash: Sha256Digest::new(expected_hash)?,
            decision,
            actor_id: ActorId::new(actor_id)?,
            reason,
        })?)
    }

    /// Find a material stage's current output for browser resume/status views.
    pub fn stage_output(
        &self,
        run_id: RunId,
        stage_id: &str,
    ) -> Result<Option<StageOutput>, AppError> {
        Ok(self
            .store
            .find_stage_output(run_id, &StageId::new(stage_id)?)?)
    }

    /// List approval history for a run.
    pub fn approvals(&self, run_id: RunId) -> Result<Vec<Approval>, AppError> {
        Ok(self.store.list_approvals(run_id)?)
    }

    /// List event history for a run.
    pub fn events(&self, run_id: RunId) -> Result<Vec<RunEvent>, AppError> {
        Ok(self.store.list_events(run_id)?)
    }

    /// Verify the tamper-evident event chain.
    pub fn verify_events(&self, run_id: RunId) -> Result<(), AppError> {
        Ok(self.store.verify_event_chain(run_id)?)
    }

    /// Store a local file in the content-addressed artifact store.
    pub fn put_artifact(
        &self,
        source: &Path,
        media_type: Option<String>,
    ) -> Result<ArtifactRecord, AppError> {
        if !source.is_file() {
            return Err(AppError::InvalidArtifact(source.to_path_buf()));
        }
        Ok(self.store.put_file(source, media_type)?)
    }

    /// Load artifact metadata.
    pub fn artifact(&self, digest: &str) -> Result<ArtifactRecord, AppError> {
        Ok(self.store.get_artifact(&Sha256Digest::new(digest)?)?)
    }

    /// Read and integrity-check raw artifact bytes within a hard byte limit.
    pub fn read_artifact_bounded(
        &self,
        digest: &str,
        maximum_bytes: u64,
    ) -> Result<Vec<u8>, AppError> {
        Ok(self
            .store
            .read_artifact_bounded(&Sha256Digest::new(digest)?, maximum_bytes)?)
    }

    /// Canonically serialize a typed JSON value into the content-addressed artifact store.
    ///
    /// This only persists the artifact. It does not claim the hash as a run stage output;
    /// use [`Self::record_derived_stage_output`] when predecessor approval is required.
    pub fn save_json_artifact<T: Serialize>(&self, value: &T) -> Result<ArtifactRecord, AppError> {
        Ok(self.store.put_json(value, None)?)
    }

    /// Load and deserialize a JSON artifact after an integrity-checked, bounded read.
    pub fn load_json_artifact_bounded<T: DeserializeOwned>(
        &self,
        digest: &str,
        maximum_bytes: u64,
    ) -> Result<T, AppError> {
        Ok(self
            .store
            .read_json_bounded(&Sha256Digest::new(digest)?, maximum_bytes)?)
    }

    /// Persist a derived JSON value and record it as a non-platform stage output.
    ///
    /// The exact predecessor output must still be approved. Built-in platform stages remain
    /// reserved for their typed application use cases.
    pub fn record_derived_stage_output<T: Serialize>(
        &self,
        run_id: RunId,
        predecessor_stage_id: &str,
        predecessor_output_hash: &str,
        stage_id: &str,
        value: &T,
    ) -> Result<Sha256Digest, AppError> {
        let stage_id = StageId::new(stage_id)?;
        if RESERVED_STAGE_IDS.contains(&stage_id.as_str()) {
            return Err(AppError::ReservedStage(stage_id.to_string()));
        }
        let predecessor_stage_id = StageId::new(predecessor_stage_id)?;
        let predecessor_output_hash = Sha256Digest::new(predecessor_output_hash)?;
        self.store.require_approved_stage(
            run_id,
            &predecessor_stage_id,
            &predecessor_output_hash,
        )?;
        let artifact = self
            .store
            .put_json(value, Some(format!("{stage_id}.json")))?;
        self.store
            .record_stage_output(run_id, stage_id, artifact.sha256.clone())?;
        Ok(artifact.sha256)
    }

    /// Complete a run from an exact approved derived terminal-stage output.
    pub fn complete_derived_run(
        &self,
        run_id: RunId,
        final_stage_id: &str,
        final_output_hash: &str,
    ) -> Result<Run, AppError> {
        let final_stage_id = StageId::new(final_stage_id)?;
        if RESERVED_STAGE_IDS.contains(&final_stage_id.as_str()) {
            return Err(AppError::ReservedStage(final_stage_id.to_string()));
        }
        if !DERIVED_TERMINAL_STAGE_IDS.contains(&final_stage_id.as_str()) {
            return Err(AppError::InvalidDerivedTerminalStage(
                final_stage_id.to_string(),
            ));
        }
        let final_output_hash = Sha256Digest::new(final_output_hash)?;
        Ok(self
            .store
            .complete_run(run_id, &final_stage_id, &final_output_hash)?)
    }

    /// Inventory a run's registered source and record the exact result for approval.
    pub fn scan_intake(&self, run_id: RunId, policy: &IntakePolicy) -> Result<Inventory, AppError> {
        let run = self.store.get_run(run_id)?;
        let project = self.store.get_project(&run.project_id)?;
        let engine = IntakeEngine::new(self.store.clone());
        let inventory = engine.scan(run_id, project.id, &project.source_path, policy)?;
        let inventory = self.store.save_inventory(&inventory)?;
        self.store.record_stage_output(
            run_id,
            StageId::new("inventory")?,
            inventory.inventory_hash.clone(),
        )?;
        Ok(inventory)
    }

    /// Load one persisted source inventory.
    pub fn inventory(&self, inventory_id: InventoryId) -> Result<Inventory, AppError> {
        Ok(self.store.get_inventory(inventory_id)?)
    }

    /// List source inventories created for a run.
    pub fn inventories(&self, run_id: RunId) -> Result<Vec<Inventory>, AppError> {
        Ok(self.store.list_inventories(run_id)?)
    }

    /// Create a source snapshot only after approval of the exact inventory hash.
    pub fn create_snapshot(
        &self,
        run_id: RunId,
        inventory_id: InventoryId,
    ) -> Result<SourceSnapshot, AppError> {
        let inventory = self.store.get_inventory(inventory_id)?;
        if inventory.run_id != run_id {
            return Err(AppError::InventoryRunMismatch {
                inventory_id,
                expected_run: run_id,
                actual_run: inventory.run_id,
            });
        }
        self.store.require_approved_stage(
            run_id,
            &StageId::new("inventory")?,
            &inventory.inventory_hash,
        )?;
        let engine = IntakeEngine::new(self.store.clone());
        let snapshot = engine.build_snapshot(&inventory)?;
        let snapshot = self.store.save_snapshot(&snapshot, run_id, inventory_id)?;
        self.store.record_stage_output(
            run_id,
            StageId::new("source-snapshot")?,
            snapshot.id.clone(),
        )?;
        Ok(snapshot)
    }

    /// Load one immutable source snapshot.
    pub fn snapshot(&self, snapshot_id: &str) -> Result<SourceSnapshot, AppError> {
        Ok(self.store.get_snapshot(&Sha256Digest::new(snapshot_id)?)?)
    }

    /// List source snapshots attached to a run.
    pub fn snapshots(&self, run_id: RunId) -> Result<Vec<SourceSnapshot>, AppError> {
        Ok(self.store.list_snapshots(run_id)?)
    }

    /// Build a raw graph only from an exact approved source snapshot.
    pub fn build_graph(
        &self,
        run_id: RunId,
        snapshot_id: &str,
        policy: &GraphPolicy,
    ) -> Result<RawProjectGraph, AppError> {
        let snapshot_id = Sha256Digest::new(snapshot_id)?;
        self.store.require_run_snapshot(run_id, &snapshot_id)?;
        self.store.require_approved_stage(
            run_id,
            &StageId::new("source-snapshot")?,
            &snapshot_id,
        )?;
        let snapshot = self.store.get_snapshot(&snapshot_id)?;
        let engine = AuditEngine::new(self.store.clone());
        let graph = engine.build_graph(&snapshot, policy)?;
        let graph = self.store.save_graph(&graph, run_id)?;
        self.store
            .record_stage_output(run_id, StageId::new("raw-graph")?, graph.id.clone())?;
        Ok(graph)
    }

    /// Load one persisted raw project graph.
    pub fn graph(&self, graph_id: &str) -> Result<RawProjectGraph, AppError> {
        Ok(self.store.get_graph(&Sha256Digest::new(graph_id)?)?)
    }

    /// List raw project graphs attached to a run.
    pub fn graphs(&self, run_id: RunId) -> Result<Vec<RawProjectGraph>, AppError> {
        Ok(self.store.list_graphs(run_id)?)
    }

    /// Run generic policies only against an exact approved raw graph.
    pub fn run_generic_audit(
        &self,
        run_id: RunId,
        graph_id: &str,
    ) -> Result<AuditReport, AppError> {
        let graph_id = Sha256Digest::new(graph_id)?;
        self.store.require_run_graph(run_id, &graph_id)?;
        self.store
            .require_approved_stage(run_id, &StageId::new("raw-graph")?, &graph_id)?;
        let graph = self.store.get_graph(&graph_id)?;
        let engine = AuditEngine::new(self.store.clone());
        let report = engine.audit(&graph)?;
        let report = self.store.save_report(&report, run_id)?;
        self.store.record_stage_output(
            run_id,
            StageId::new("generic-audit")?,
            report.id.clone(),
        )?;
        Ok(report)
    }

    /// Load one persisted generic audit report.
    pub fn report(&self, report_id: &str) -> Result<AuditReport, AppError> {
        Ok(self.store.get_report(&Sha256Digest::new(report_id)?)?)
    }

    /// List generic audit reports attached to a run.
    pub fn reports(&self, run_id: RunId) -> Result<Vec<AuditReport>, AppError> {
        Ok(self.store.list_reports(run_id)?)
    }

    /// Finish a generic-only governed run after approval of its exact audit report.
    pub fn complete_generic_run(&self, run_id: RunId, report_id: &str) -> Result<Run, AppError> {
        let report_id = Sha256Digest::new(report_id)?;
        let belongs_to_run = self
            .store
            .list_reports(run_id)?
            .iter()
            .any(|report| report.id == report_id);
        if !belongs_to_run {
            return Err(StoreError::NotFound {
                kind: "run report binding",
                id: format!("{run_id}/{report_id}"),
            }
            .into());
        }
        Ok(self
            .store
            .complete_run(run_id, &StageId::new("generic-audit")?, &report_id)?)
    }

    /// Validate a declarative adapter and persist its pass/fail conformance report without install.
    pub fn check_adapter(&self, source: &Path) -> Result<AdapterConformanceReport, AppError> {
        let prepared = self.prepare_adapter(source)?;
        Ok(self.store.save_adapter_conformance(&prepared.conformance)?)
    }

    /// Validate and install one immutable declarative adapter JSON package.
    pub fn install_adapter(&self, source: &Path) -> Result<AdapterRecord, AppError> {
        let prepared = self.prepare_adapter(source)?;
        let conformance = self.store.save_adapter_conformance(&prepared.conformance)?;
        if !conformance.passed {
            return Err(AppError::AdapterConformanceFailed {
                report_id: conformance.id,
            });
        }

        let source_artifact = self.store.put_bytes(
            &prepared.bytes,
            source
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Some("application/json".to_owned()),
        )?;
        Ok(self.store.save_adapter(&AdapterRecord {
            package_hash: prepared.package_hash,
            source_hash: source_artifact.sha256,
            conformance_report_id: conformance.id,
            package: prepared.package,
            installed_at: Utc::now(),
        })?)
    }

    /// Load one installed immutable adapter by package hash.
    pub fn adapter(&self, package_hash: &str) -> Result<AdapterRecord, AppError> {
        Ok(self.store.get_adapter(&Sha256Digest::new(package_hash)?)?)
    }

    /// List installed immutable adapter packages.
    pub fn adapters(&self) -> Result<Vec<AdapterRecord>, AppError> {
        let maximum = AdapterEngine::new(self.store.clone())
            .limits()
            .max_registry_packages;
        Ok(self.store.list_adapters_bounded(maximum)?)
    }

    /// Load the conformance certificate for an installed adapter package.
    pub fn adapter_conformance(
        &self,
        report_id: &str,
    ) -> Result<AdapterConformanceReport, AppError> {
        Ok(self
            .store
            .get_adapter_conformance(&Sha256Digest::new(report_id)?)?)
    }

    /// Detect installed adapters only against an exact approved raw graph.
    pub fn detect_adapters(
        &self,
        run_id: RunId,
        graph_id: &str,
    ) -> Result<AdapterDetectionReport, AppError> {
        let graph_id = Sha256Digest::new(graph_id)?;
        self.store.require_run_graph(run_id, &graph_id)?;
        self.store
            .require_approved_stage(run_id, &StageId::new("raw-graph")?, &graph_id)?;
        let graph = self.store.get_graph(&graph_id)?;
        let engine = AdapterEngine::new(self.store.clone());
        let records = self
            .store
            .list_adapters_bounded(engine.limits().max_registry_packages)?;
        let detection = engine.detect(&graph, &records)?;
        let detection = self.store.save_adapter_detection(run_id, &detection)?;
        self.store.record_stage_output(
            run_id,
            StageId::new("adapter-detection")?,
            detection.id.clone(),
        )?;
        Ok(detection)
    }

    /// Load one persisted adapter detection report.
    pub fn adapter_detection(
        &self,
        detection_id: &str,
    ) -> Result<AdapterDetectionReport, AppError> {
        Ok(self
            .store
            .get_adapter_detection(&Sha256Digest::new(detection_id)?)?)
    }

    /// List adapter detection reports attached to a run.
    pub fn adapter_detections(
        &self,
        run_id: RunId,
    ) -> Result<Vec<AdapterDetectionReport>, AppError> {
        Ok(self.store.list_adapter_detections(run_id)?)
    }

    /// Lock exact detected adapter packages only after detection approval.
    pub fn create_adapter_plan(
        &self,
        run_id: RunId,
        detection_id: &str,
        package_hashes: &[String],
    ) -> Result<AdapterPlan, AppError> {
        let detection_id = Sha256Digest::new(detection_id)?;
        self.store
            .require_run_adapter_detection(run_id, &detection_id)?;
        self.store.require_approved_stage(
            run_id,
            &StageId::new("adapter-detection")?,
            &detection_id,
        )?;
        let detection = self.store.get_adapter_detection(&detection_id)?;
        let selected = package_hashes
            .iter()
            .map(Sha256Digest::new)
            .collect::<Result<Vec<_>, _>>()?;
        let records = selected
            .iter()
            .map(|hash| self.store.get_adapter(hash))
            .collect::<Result<Vec<_>, _>>()?;
        let engine = AdapterEngine::new(self.store.clone());
        let plan = engine.create_plan(&detection, &selected, &records)?;
        let plan = self.store.save_adapter_plan(run_id, &plan)?;
        self.store
            .record_stage_output(run_id, StageId::new("adapter-plan")?, plan.id.clone())?;
        Ok(plan)
    }

    /// Load one immutable adapter selection plan.
    pub fn adapter_plan(&self, plan_id: &str) -> Result<AdapterPlan, AppError> {
        Ok(self.store.get_adapter_plan(&Sha256Digest::new(plan_id)?)?)
    }

    /// List immutable adapter plans attached to a run.
    pub fn adapter_plans(&self, run_id: RunId) -> Result<Vec<AdapterPlan>, AppError> {
        Ok(self.store.list_adapter_plans(run_id)?)
    }

    /// Build a semantic graph only from an exact approved adapter plan.
    pub fn build_semantic_graph(
        &self,
        run_id: RunId,
        plan_id: &str,
    ) -> Result<SemanticGraph, AppError> {
        let plan_id = Sha256Digest::new(plan_id)?;
        self.store.require_run_adapter_plan(run_id, &plan_id)?;
        self.store
            .require_approved_stage(run_id, &StageId::new("adapter-plan")?, &plan_id)?;
        let plan = self.store.get_adapter_plan(&plan_id)?;
        let raw_graph = self.store.get_graph(&plan.raw_graph_id)?;
        let snapshot = self.store.get_snapshot(&raw_graph.snapshot_id)?;
        let records = self.adapter_records_for_plan(&plan)?;
        let engine = AdapterEngine::new(self.store.clone());
        let graph = engine.project_with_store(&raw_graph, &snapshot, &plan, &records)?;
        let graph = self.store.save_semantic_graph(run_id, &graph)?;
        self.store.record_stage_output(
            run_id,
            StageId::new("semantic-graph")?,
            graph.id.clone(),
        )?;
        Ok(graph)
    }

    /// Load one adapter-produced semantic graph.
    pub fn semantic_graph(&self, graph_id: &str) -> Result<SemanticGraph, AppError> {
        Ok(self
            .store
            .get_semantic_graph(&Sha256Digest::new(graph_id)?)?)
    }

    /// List semantic graphs attached to a run.
    pub fn semantic_graphs(&self, run_id: RunId) -> Result<Vec<SemanticGraph>, AppError> {
        Ok(self.store.list_semantic_graphs(run_id)?)
    }

    /// Evaluate semantic policies only against an exact approved semantic graph.
    pub fn run_semantic_audit(
        &self,
        run_id: RunId,
        graph_id: &str,
    ) -> Result<SemanticAuditReport, AppError> {
        let graph_id = Sha256Digest::new(graph_id)?;
        self.store.require_run_semantic_graph(run_id, &graph_id)?;
        self.store
            .require_approved_stage(run_id, &StageId::new("semantic-graph")?, &graph_id)?;
        let graph = self.store.get_semantic_graph(&graph_id)?;
        let plan = self.store.get_adapter_plan(&graph.adapter_plan_id)?;
        let records = self.adapter_records_for_plan(&plan)?;
        let engine = AdapterEngine::new(self.store.clone());
        let report = engine.audit(&graph, &plan, &records)?;
        let report = self.store.save_semantic_report(run_id, &report)?;
        self.store.record_stage_output(
            run_id,
            StageId::new("semantic-audit")?,
            report.id.clone(),
        )?;
        Ok(report)
    }

    /// Load one adapter semantic audit report.
    pub fn semantic_report(&self, report_id: &str) -> Result<SemanticAuditReport, AppError> {
        Ok(self
            .store
            .get_semantic_report(&Sha256Digest::new(report_id)?)?)
    }

    /// List semantic audit reports attached to a run.
    pub fn semantic_reports(&self, run_id: RunId) -> Result<Vec<SemanticAuditReport>, AppError> {
        Ok(self.store.list_semantic_reports(run_id)?)
    }

    /// Resolve named views only after exact semantic graph and audit approvals.
    pub fn build_projection_catalog(
        &self,
        run_id: RunId,
        graph_id: &str,
        report_id: &str,
    ) -> Result<ProjectionCatalog, AppError> {
        let graph_id = Sha256Digest::new(graph_id)?;
        let report_id = Sha256Digest::new(report_id)?;
        self.store.require_run_semantic_graph(run_id, &graph_id)?;
        self.store.require_run_semantic_report(run_id, &report_id)?;
        self.store
            .require_approved_stage(run_id, &StageId::new("semantic-graph")?, &graph_id)?;
        self.store
            .require_approved_stage(run_id, &StageId::new("semantic-audit")?, &report_id)?;
        let graph = self.store.get_semantic_graph(&graph_id)?;
        let report = self.store.get_semantic_report(&report_id)?;
        let plan = self.store.get_adapter_plan(&graph.adapter_plan_id)?;
        let records = self.adapter_records_for_plan(&plan)?;
        let engine = AdapterEngine::new(self.store.clone());
        let catalog = engine.resolve_catalog(&graph, &report, &plan, &records)?;
        let catalog = self.store.save_projection_catalog(run_id, &catalog)?;
        self.store.record_stage_output(
            run_id,
            StageId::new("projection-catalog")?,
            catalog.id.clone(),
        )?;
        Ok(catalog)
    }

    /// Load one resolved semantic projection catalog.
    pub fn projection_catalog(&self, catalog_id: &str) -> Result<ProjectionCatalog, AppError> {
        Ok(self
            .store
            .get_projection_catalog(&Sha256Digest::new(catalog_id)?)?)
    }

    /// List resolved projection catalogs attached to a run.
    pub fn projection_catalogs(&self, run_id: RunId) -> Result<Vec<ProjectionCatalog>, AppError> {
        Ok(self.store.list_projection_catalogs(run_id)?)
    }

    fn adapter_records_for_plan(&self, plan: &AdapterPlan) -> Result<Vec<AdapterRecord>, AppError> {
        plan.adapters
            .iter()
            .map(|locked| {
                self.store
                    .get_adapter(&locked.package_hash)
                    .map_err(Into::into)
            })
            .collect()
    }

    fn prepare_adapter(&self, source: &Path) -> Result<PreparedAdapter, AppError> {
        let file = File::open(source).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                AppError::InvalidArtifact(source.to_path_buf())
            } else {
                AppError::Io(error)
            }
        })?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(AppError::InvalidArtifact(source.to_path_buf()));
        }
        let engine = AdapterEngine::new(self.store.clone());
        let maximum = engine.limits().max_package_bytes;
        let maximum_u64 = u64::try_from(maximum).unwrap_or(u64::MAX);
        if metadata.len() > maximum_u64 {
            return Err(AppError::AdapterPackageTooLarge {
                path: source.to_path_buf(),
                size: metadata.len(),
                maximum: maximum_u64,
            });
        }
        let mut bytes = Vec::with_capacity(maximum.min(64 * 1024));
        file.take(maximum_u64.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() > maximum {
            return Err(AppError::AdapterPackageTooLarge {
                path: source.to_path_buf(),
                size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                maximum: maximum_u64,
            });
        }
        let package = engine.parse_package_json(&bytes)?;
        let package_hash = engine.package_hash(&package)?;
        let conformance = engine.conformance_report(&package)?;
        Ok(PreparedAdapter {
            bytes,
            package,
            package_hash,
            conformance,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use qbm_domain::{Decision, GraphPolicy, IntakePolicy, RunMode, RunState, StageState};
    use qbm_store::StoreError;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    use super::*;

    #[derive(Debug, Serialize)]
    struct ForwardArtifactPayload {
        alpha: String,
        omega: Vec<u8>,
    }

    #[derive(Debug, Serialize)]
    struct ReverseArtifactPayload {
        omega: Vec<u8>,
        alpha: String,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct LoadedArtifactPayload {
        alpha: String,
        omega: Vec<u8>,
    }

    #[test]
    fn application_lifecycle_preserves_source_and_records_decision() {
        let data = TempDir::new().unwrap();
        let source = TempDir::new().unwrap();
        let app = PlatformApp::open(data.path()).unwrap();
        app.register_project("demo", "Demo", source.path()).unwrap();
        let run = app.start_run("demo", RunMode::Governed).unwrap();
        let hash = "c".repeat(64);
        app.complete_stage(run.id, "custom-review", &hash).unwrap();
        app.decide_stage(
            run.id,
            "custom-review",
            &hash,
            Decision::Approve,
            "owner",
            Some("reviewed".to_owned()),
        )
        .unwrap();
        assert_eq!(app.run(run.id).unwrap().state, RunState::Running);
        assert_eq!(app.approvals(run.id).unwrap().len(), 1);
        app.verify_events(run.id).unwrap();
        assert!(source.path().is_dir());
    }

    #[test]
    fn typed_json_artifacts_are_canonical_and_bounded() {
        let data = TempDir::new().unwrap();
        let app = PlatformApp::open(data.path()).unwrap();
        let forward = app
            .save_json_artifact(&ForwardArtifactPayload {
                alpha: "stable".to_owned(),
                omega: vec![3, 2, 1],
            })
            .unwrap();
        let reverse = app
            .save_json_artifact(&ReverseArtifactPayload {
                omega: vec![3, 2, 1],
                alpha: "stable".to_owned(),
            })
            .unwrap();

        assert_eq!(forward.sha256, reverse.sha256);
        assert_eq!(forward.media_type.as_deref(), Some("application/json"));
        assert_eq!(
            app.read_artifact_bounded(forward.sha256.as_str(), forward.size_bytes)
                .unwrap(),
            br#"{"alpha":"stable","omega":[3,2,1]}"#
        );
        let loaded: LoadedArtifactPayload = app
            .load_json_artifact_bounded(forward.sha256.as_str(), forward.size_bytes)
            .unwrap();
        assert_eq!(
            loaded,
            LoadedArtifactPayload {
                alpha: "stable".to_owned(),
                omega: vec![3, 2, 1],
            }
        );
        assert!(matches!(
            app.load_json_artifact_bounded::<LoadedArtifactPayload>(
                forward.sha256.as_str(),
                forward.size_bytes - 1,
            ),
            Err(AppError::Store(StoreError::ArtifactReadLimit {
                digest,
                maximum_bytes,
                ..
            })) if digest == forward.sha256 && maximum_bytes == forward.size_bytes - 1
        ));
    }

    #[test]
    fn derived_stage_output_requires_exact_predecessor_approval() {
        let data = TempDir::new().unwrap();
        let source = TempDir::new().unwrap();
        let app = PlatformApp::open(data.path()).unwrap();
        app.register_project("derived", "Derived artifacts", source.path())
            .unwrap();
        let run = app.start_run("derived", RunMode::Governed).unwrap();
        let predecessor_hash = "a".repeat(64);
        app.complete_stage(run.id, "profile", &predecessor_hash)
            .unwrap();
        let value = ForwardArtifactPayload {
            alpha: "profile-diff".to_owned(),
            omega: vec![1, 4, 9],
        };

        assert!(matches!(
            app.record_derived_stage_output(
                run.id,
                "profile",
                &predecessor_hash,
                "profile-diff",
                &value,
            ),
            Err(AppError::Store(StoreError::RequiredApprovalMissing {
                stage_id,
                expected_hash,
                ..
            })) if stage_id.as_str() == "profile"
                && expected_hash.as_str() == predecessor_hash
        ));
        assert!(app.stage_output(run.id, "profile-diff").unwrap().is_none());

        app.decide_stage(
            run.id,
            "profile",
            &predecessor_hash,
            Decision::Approve,
            "owner",
            None,
        )
        .unwrap();
        assert!(matches!(
            app.record_derived_stage_output(
                run.id,
                "profile",
                &"b".repeat(64),
                "profile-diff",
                &value,
            ),
            Err(AppError::Store(StoreError::RequiredApprovalMissing { .. }))
        ));

        let artifact_hash = app
            .record_derived_stage_output(
                run.id,
                "profile",
                &predecessor_hash,
                "profile-diff",
                &value,
            )
            .unwrap();
        let output = app.stage_output(run.id, "profile-diff").unwrap().unwrap();
        assert_eq!(output.output_hash, artifact_hash);
        assert_eq!(output.state, StageState::WaitingApproval);
        let artifact = app.artifact(artifact_hash.as_str()).unwrap();
        let loaded: LoadedArtifactPayload = app
            .load_json_artifact_bounded(artifact_hash.as_str(), artifact.size_bytes)
            .unwrap();
        assert_eq!(loaded.alpha, "profile-diff");

        for stage in RESERVED_STAGE_IDS {
            assert!(matches!(
                app.record_derived_stage_output(
                    run.id,
                    "profile",
                    &predecessor_hash,
                    stage,
                    &value,
                ),
                Err(AppError::ReservedStage(rejected)) if rejected == *stage
            ));
        }

        app.verify_events(run.id).unwrap();

        let reopened = PlatformApp::open(data.path()).unwrap();
        let reopened_value: LoadedArtifactPayload = reopened
            .load_json_artifact_bounded(artifact_hash.as_str(), artifact.size_bytes)
            .unwrap();
        assert_eq!(reopened_value, loaded);
    }

    #[test]
    fn derived_run_completion_requires_an_approved_full_report() {
        let data = TempDir::new().unwrap();
        let source = TempDir::new().unwrap();
        let app = PlatformApp::open(data.path()).unwrap();
        app.register_project("completion", "Derived completion", source.path())
            .unwrap();
        let run = app.start_run("completion", RunMode::Governed).unwrap();
        let predecessor_hash = "e".repeat(64);
        app.complete_stage(run.id, "profile-diff", &predecessor_hash)
            .unwrap();
        app.decide_stage(
            run.id,
            "profile-diff",
            &predecessor_hash,
            Decision::Approve,
            "owner",
            None,
        )
        .unwrap();
        let final_hash = app
            .record_derived_stage_output(
                run.id,
                "profile-diff",
                &predecessor_hash,
                "full-report",
                &ForwardArtifactPayload {
                    alpha: "full-report".to_owned(),
                    omega: vec![2, 7, 1, 8],
                },
            )
            .unwrap();

        assert!(matches!(
            app.complete_derived_run(run.id, "full-report", final_hash.as_str()),
            Err(AppError::Store(StoreError::RequiredApprovalMissing { .. }))
        ));
        assert!(matches!(
            app.complete_derived_run(run.id, "profile-diff", &predecessor_hash),
            Err(AppError::InvalidDerivedTerminalStage(stage)) if stage == "profile-diff"
        ));
        assert!(matches!(
            app.complete_derived_run(run.id, "generic-audit", final_hash.as_str()),
            Err(AppError::ReservedStage(stage)) if stage == "generic-audit"
        ));

        app.decide_stage(
            run.id,
            "full-report",
            final_hash.as_str(),
            Decision::Approve,
            "owner",
            None,
        )
        .unwrap();
        let completed = app
            .complete_derived_run(run.id, "full-report", final_hash.as_str())
            .unwrap();
        assert_eq!(completed.state, RunState::Complete);
        assert_eq!(
            app.complete_derived_run(run.id, "full-report", final_hash.as_str())
                .unwrap()
                .state,
            RunState::Complete
        );
        app.verify_events(run.id).unwrap();

        let reopened = PlatformApp::open(data.path()).unwrap();
        assert_eq!(reopened.run(run.id).unwrap().state, RunState::Complete);
    }

    #[test]
    fn generic_stage_completion_cannot_fabricate_platform_outputs() {
        let data = TempDir::new().unwrap();
        let source = TempDir::new().unwrap();
        let app = PlatformApp::open(data.path()).unwrap();
        app.register_project("demo", "Demo", source.path()).unwrap();
        let run = app.start_run("demo", RunMode::Governed).unwrap();

        for stage in RESERVED_STAGE_IDS {
            assert!(matches!(
                app.complete_stage(run.id, stage, &"d".repeat(64)),
                Err(AppError::ReservedStage(rejected)) if rejected == *stage
            ));
        }
        assert!(app.events(run.id).unwrap().iter().all(|event| {
            event
                .stage_id
                .as_ref()
                .is_none_or(|stage| !RESERVED_STAGE_IDS.contains(&stage.as_str()))
        }));
    }

    #[test]
    fn approved_inventory_becomes_a_durable_frozen_snapshot() {
        let data = TempDir::new().unwrap();
        let source = TempDir::new().unwrap();
        fs::write(source.path().join("rules.yaml"), b"version: 1\n").unwrap();
        let app = PlatformApp::open(data.path()).unwrap();
        app.register_project("pathex", "PATHEX", source.path())
            .unwrap();
        let run = app.start_run("pathex", RunMode::Governed).unwrap();
        let inventory = app.scan_intake(run.id, &IntakePolicy::default()).unwrap();

        assert!(matches!(
            app.create_snapshot(run.id, inventory.id),
            Err(AppError::Store(StoreError::RequiredApprovalMissing { .. }))
        ));
        app.decide_stage(
            run.id,
            "inventory",
            inventory.inventory_hash.as_str(),
            Decision::Approve,
            "owner",
            Some("inventory reviewed".to_owned()),
        )
        .unwrap();

        fs::write(source.path().join("rules.yaml"), b"version: 2 changed\n").unwrap();
        let snapshot = app.create_snapshot(run.id, inventory.id).unwrap();
        assert_eq!(snapshot.total_bytes, b"version: 1\n".len() as u64);
        assert_eq!(snapshot.artifacts.len(), 1);

        let reopened = PlatformApp::open(data.path()).unwrap();
        assert_eq!(reopened.inventories(run.id).unwrap().len(), 1);
        assert_eq!(reopened.snapshots(run.id).unwrap(), vec![snapshot]);
        reopened.verify_events(run.id).unwrap();
    }

    #[test]
    fn blocked_inventory_cannot_be_snapshotted_after_approval() {
        let data = TempDir::new().unwrap();
        let source = TempDir::new().unwrap();
        fs::write(source.path().join("model.bin"), b"large").unwrap();
        let app = PlatformApp::open(data.path()).unwrap();
        app.register_project("model", "Model", source.path())
            .unwrap();
        let run = app.start_run("model", RunMode::Governed).unwrap();
        let policy = IntakePolicy {
            max_single_file_bytes: 4,
            ..IntakePolicy::default()
        };
        let inventory = app.scan_intake(run.id, &policy).unwrap();
        app.decide_stage(
            run.id,
            "inventory",
            inventory.inventory_hash.as_str(),
            Decision::Approve,
            "owner",
            None,
        )
        .unwrap();
        assert!(matches!(
            app.create_snapshot(run.id, inventory.id),
            Err(AppError::Intake(IntakeError::BlockedInventory { .. }))
        ));
    }

    #[test]
    fn graph_and_audit_each_require_exact_predecessor_approval() {
        let data = TempDir::new().unwrap();
        let source = TempDir::new().unwrap();
        fs::write(
            source.path().join("rules.yaml"),
            b"definitions:\n  one:\n    id: same\n  two:\n    id: same\nflow:\n  ref: missing\n",
        )
        .unwrap();
        let app = PlatformApp::open(data.path()).unwrap();
        app.register_project("audit", "Audit fixture", source.path())
            .unwrap();
        let run = app.start_run("audit", RunMode::Governed).unwrap();
        let inventory = app.scan_intake(run.id, &IntakePolicy::default()).unwrap();
        app.decide_stage(
            run.id,
            "inventory",
            inventory.inventory_hash.as_str(),
            Decision::Approve,
            "owner",
            None,
        )
        .unwrap();
        let snapshot = app.create_snapshot(run.id, inventory.id).unwrap();
        assert!(matches!(
            app.build_graph(run.id, snapshot.id.as_str(), &GraphPolicy::default()),
            Err(AppError::Store(StoreError::RequiredApprovalMissing { .. }))
        ));
        app.decide_stage(
            run.id,
            "source-snapshot",
            snapshot.id.as_str(),
            Decision::Approve,
            "owner",
            None,
        )
        .unwrap();
        let graph = app
            .build_graph(run.id, snapshot.id.as_str(), &GraphPolicy::default())
            .unwrap();
        assert!(matches!(
            app.run_generic_audit(run.id, graph.id.as_str()),
            Err(AppError::Store(StoreError::RequiredApprovalMissing { .. }))
        ));
        app.decide_stage(
            run.id,
            "raw-graph",
            graph.id.as_str(),
            Decision::Approve,
            "owner",
            None,
        )
        .unwrap();
        let report = app.run_generic_audit(run.id, graph.id.as_str()).unwrap();
        assert!(report.summary.errors >= 2);

        let reopened = PlatformApp::open(data.path()).unwrap();
        assert_eq!(reopened.graphs(run.id).unwrap(), vec![graph]);
        assert_eq!(reopened.reports(run.id).unwrap(), vec![report]);
        reopened.verify_events(run.id).unwrap();
    }

    #[test]
    fn invalid_adapter_package_does_not_mutate_the_registry() {
        let data = TempDir::new().unwrap();
        let package = data.path().join("invalid-adapter.json");
        fs::write(&package, b"{not valid JSON").unwrap();
        let app = PlatformApp::open(data.path().join("platform")).unwrap();

        assert!(matches!(
            app.install_adapter(&package),
            Err(AppError::Adapter(_))
        ));
        assert!(app.adapters().unwrap().is_empty());
    }

    #[test]
    fn adapter_check_persists_failure_without_installing_package() {
        let data = TempDir::new().unwrap();
        let package = data.path().join("nonconformant-adapter.json");
        fs::write(
            &package,
            br#"{
                "schema_version":"qbm.adapter-package/v1",
                "manifest":{
                    "schema_version":"qbm.adapter-manifest/v1",
                    "adapter_api_version":"qbm.adapter-api/v1",
                    "id":"empty-example",
                    "version":"1.0.0",
                    "display_name":"Empty example",
                    "description":"Intentionally incomplete fixture",
                    "minimum_platform_schema":"qbm.platform/v1",
                    "capabilities":[],
                    "license":"Apache-2.0"
                },
                "detection":[],
                "mappings":{"schema_version":"qbm.mapping-pack/v1","nodes":[],"edges":[]},
                "policies":{"schema_version":"qbm.policy-pack/v1","rules":[]},
                "projections":[],
                "fixtures":[]
            }"#,
        )
        .unwrap();
        let app = PlatformApp::open(data.path().join("platform")).unwrap();

        let report = app.check_adapter(&package).unwrap();
        assert!(!report.passed);
        assert_eq!(app.adapter_conformance(report.id.as_str()).unwrap(), report);
        assert!(app.adapters().unwrap().is_empty());
        assert!(matches!(
            app.install_adapter(&package),
            Err(AppError::AdapterConformanceFailed { .. })
        ));
        assert!(app.adapters().unwrap().is_empty());
    }

    #[test]
    fn oversized_adapter_package_is_rejected_before_reading() {
        let data = TempDir::new().unwrap();
        let package = data.path().join("oversized-adapter.json");
        let file = fs::File::create(&package).unwrap();
        file.set_len(2 * 1024 * 1024 + 1).unwrap();
        let app = PlatformApp::open(data.path().join("platform")).unwrap();

        assert!(matches!(
            app.install_adapter(&package),
            Err(AppError::AdapterPackageTooLarge { .. })
        ));
        assert!(app.adapters().unwrap().is_empty());
    }
}
