//! Durable local metadata, event, approval and artifact storage.

use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};

use chrono::{DateTime, SecondsFormat, Utc};
use qbm_canonical::{CanonicalError, canonical_json, hash_value, sha256_bytes, sha256_reader};
use qbm_domain::{
    ActorId, AdapterConformanceReport, AdapterDetectionReport, AdapterPlan, AdapterRecord,
    Approval, ApprovalId, ApprovalRequest, ArtifactRecord, AuditReport, DOMAIN_SCHEMA_VERSION,
    Decision, EventId, Inventory, InventoryId, Project, ProjectId, ProjectionCatalog,
    RawProjectGraph, Run, RunEvent, RunId, RunMode, RunState, SemanticAuditReport, SemanticGraph,
    Sha256Digest, SourceAcquisition, SourceAcquisitionKind, SourceSnapshot, StageId, StageOutput,
    StageState,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const DATABASE_FILE: &str = "qbm.sqlite";
const DATABASE_SCHEMA_VERSION: i64 = 5;

/// Persistence and integrity errors.
#[derive(Debug, Error)]
pub enum StoreError {
    /// Filesystem operation failed.
    #[error("filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    /// `SQLite` operation failed.
    #[error("database operation failed: {0}")]
    Database(#[from] rusqlite::Error),
    /// Domain value failed validation.
    #[error(transparent)]
    Domain(#[from] qbm_domain::DomainError),
    /// Canonical hashing failed.
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    /// JSON payload could not be decoded.
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// UUID text could not be decoded.
    #[error("stored UUID is invalid: {0}")]
    Uuid(#[from] uuid::Error),
    /// Timestamp text could not be decoded.
    #[error("stored timestamp is invalid: {0}")]
    Timestamp(#[from] chrono::ParseError),
    /// Requested record was not found.
    #[error("{kind} {id:?} was not found")]
    NotFound {
        /// Record category.
        kind: &'static str,
        /// Requested identity.
        id: String,
    },
    /// A project ID already exists.
    #[error("project {0:?} already exists")]
    ProjectExists(String),
    /// An adapter ID/version is already bound to different immutable content.
    #[error(
        "adapter {adapter_id}@{version} is already installed at {existing_hash}, not {new_hash}"
    )]
    AdapterVersionConflict {
        /// Adapter ID.
        adapter_id: String,
        /// Version label.
        version: String,
        /// Existing package hash.
        existing_hash: Sha256Digest,
        /// Conflicting package hash.
        new_hash: Sha256Digest,
    },
    /// A record violates a required immutable predecessor relationship.
    #[error("invalid persisted relationship: {0}")]
    InvalidRelationship(String),
    /// An approval was attempted against an earlier stage-output hash.
    #[error("stale approval: expected stage output {expected}, but current output is {current}")]
    StaleApproval {
        /// Hash supplied by the reviewer.
        expected: Sha256Digest,
        /// Current stage-output hash.
        current: Sha256Digest,
    },
    /// Rejection and change requests require an explanation.
    #[error("{0} requires a non-empty reason")]
    ReasonRequired(Decision),
    /// A decision was attempted for a stage that is not awaiting review.
    #[error("stage {stage_id} is {state}, not waiting for approval")]
    StageNotAwaitingApproval {
        /// Stage that rejected the duplicate or out-of-order decision.
        stage_id: StageId,
        /// Current durable stage state.
        state: StageState,
    },
    /// A persisted integer cannot be represented safely.
    #[error("numeric value {value} is outside the supported range for {field}")]
    NumericRange {
        /// Field name.
        field: &'static str,
        /// Rejected value.
        value: u64,
    },
    /// The event chain is discontinuous or has been modified.
    #[error("event chain for run {run_id} is invalid at sequence {sequence}: {reason}")]
    EventChainInvalid {
        /// Affected run.
        run_id: RunId,
        /// First failing sequence.
        sequence: u64,
        /// Integrity failure.
        reason: String,
    },
    /// Artifact bytes no longer match their content identity.
    #[error("artifact {expected} failed integrity verification; calculated {calculated}")]
    ArtifactIntegrity {
        /// Identity stored in metadata.
        expected: Sha256Digest,
        /// Identity calculated from current bytes.
        calculated: Sha256Digest,
    },
    /// A bounded artifact read was refused before unbounded allocation.
    #[error("artifact {digest} is {observed_bytes} bytes; bounded read maximum is {maximum_bytes}")]
    ArtifactReadLimit {
        /// Requested artifact.
        digest: Sha256Digest,
        /// Size from trusted metadata, filesystem metadata, or the bounded read sentinel.
        observed_bytes: u64,
        /// Caller-provided hard limit.
        maximum_bytes: u64,
    },
    /// A bounded registry/list operation was refused before loading every manifest.
    #[error("{kind} contains {observed} records; bounded load maximum is {maximum}")]
    RecordReadLimit {
        /// Registry or record category.
        kind: &'static str,
        /// Count obtained by a database-only preflight.
        observed: u64,
        /// Caller-provided hard record limit.
        maximum: u64,
    },
    /// Snapshot creation requires an approved exact predecessor stage.
    #[error(
        "stage {stage_id} must be approved at {expected_hash}; current state is {actual_state}"
    )]
    RequiredApprovalMissing {
        /// Required stage.
        stage_id: StageId,
        /// Required output identity.
        expected_hash: Sha256Digest,
        /// Current stage state or `missing`.
        actual_state: String,
    },
}

/// Local store backed by `SQLite` and content-addressed files.
#[derive(Debug, Clone)]
pub struct PlatformStore {
    root: PathBuf,
    database_path: PathBuf,
    temporary_root: PathBuf,
}

#[derive(Debug, Serialize)]
struct EventIdentity<'a> {
    event_id: String,
    run_id: String,
    sequence: u64,
    kind: &'a str,
    stage_id: Option<&'a str>,
    payload: &'a Value,
    created_at: &'a str,
    previous_hash: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct SourceAcquisitionIdentity<'a> {
    schema_version: &'a str,
    project_id: &'a ProjectId,
    kind: SourceAcquisitionKind,
    source_locator: &'a str,
    repository_id: Option<u64>,
    requested_revision: Option<&'a str>,
    resolved_revision: Option<&'a str>,
    retrieval_url: Option<&'a str>,
    provider_api_version: Option<&'a str>,
    content_sha256: &'a Sha256Digest,
    total_bytes: u64,
}

impl PlatformStore {
    /// Open or initialize a platform data directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref().to_path_buf();
        let artifact_root = root.join("artifacts").join("sha256");
        let temporary_root = root.join("tmp");
        fs::create_dir_all(&artifact_root)?;
        fs::create_dir_all(&temporary_root)?;
        let store = Self {
            database_path: root.join(DATABASE_FILE),
            root,
            temporary_root,
        };
        let connection = store.connection()?;
        Self::migrate(&connection)?;
        Ok(store)
    }

    /// Root directory containing all local platform state.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path to the `SQLite` metadata database.
    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    fn connection(&self) -> Result<Connection, StoreError> {
        let connection = Connection::open(&self.database_path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(connection)
    }

    fn migrate(connection: &Connection) -> Result<(), StoreError> {
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < 1 {
            Self::migrate_v1(connection)?;
        }
        if version < 2 {
            Self::migrate_v2(connection)?;
        }
        if version < 3 {
            Self::migrate_v3(connection)?;
        }
        if version < 4 {
            Self::migrate_v4(connection)?;
        }
        if version < 5 {
            Self::migrate_v5(connection)?;
        }
        if version > DATABASE_SCHEMA_VERSION {
            return Err(StoreError::Database(rusqlite::Error::InvalidQuery));
        }
        Ok(())
    }

    fn migrate_v1(connection: &Connection) -> Result<(), StoreError> {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS projects (
                    id TEXT PRIMARY KEY,
                    display_name TEXT NOT NULL,
                    source_path TEXT NOT NULL,
                    created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS runs (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(id),
                    mode TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS runs_project_idx ON runs(project_id, created_at);
                 CREATE TABLE IF NOT EXISTS run_events (
                    run_id TEXT NOT NULL REFERENCES runs(id),
                    sequence INTEGER NOT NULL,
                    event_id TEXT NOT NULL UNIQUE,
                    kind TEXT NOT NULL,
                    stage_id TEXT,
                    payload_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    previous_hash TEXT,
                    event_hash TEXT NOT NULL UNIQUE,
                    PRIMARY KEY (run_id, sequence)
                 );
                 CREATE TABLE IF NOT EXISTS stage_outputs (
                    run_id TEXT NOT NULL REFERENCES runs(id),
                    stage_id TEXT NOT NULL,
                    output_hash TEXT NOT NULL,
                    state TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (run_id, stage_id)
                 );
                 CREATE TABLE IF NOT EXISTS approvals (
                    id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL REFERENCES runs(id),
                    stage_id TEXT NOT NULL,
                    stage_output_hash TEXT NOT NULL,
                    decision TEXT NOT NULL,
                    actor_id TEXT NOT NULL,
                    reason TEXT,
                    created_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS approvals_run_idx
                    ON approvals(run_id, stage_id, created_at);
                 CREATE TABLE IF NOT EXISTS artifacts (
                    sha256 TEXT PRIMARY KEY,
                    size_bytes INTEGER NOT NULL,
                    storage_path TEXT NOT NULL,
                    original_name TEXT,
                    media_type TEXT,
                    created_at TEXT NOT NULL
                 );
                 PRAGMA user_version = 1;
                 COMMIT;",
        )?;
        Ok(())
    }

    fn migrate_v2(connection: &Connection) -> Result<(), StoreError> {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
                 CREATE TABLE IF NOT EXISTS inventories (
                    id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL REFERENCES runs(id),
                    project_id TEXT NOT NULL REFERENCES projects(id),
                    inventory_hash TEXT NOT NULL,
                    policy_hash TEXT NOT NULL,
                    manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                    source_kind TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    UNIQUE(run_id, inventory_hash)
                 );
                 CREATE INDEX IF NOT EXISTS inventories_project_idx
                    ON inventories(project_id, created_at);
                 CREATE TABLE IF NOT EXISTS source_snapshots (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(id),
                    inventory_hash TEXT NOT NULL,
                    policy_hash TEXT NOT NULL,
                    manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                    created_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS snapshots_project_idx
                    ON source_snapshots(project_id, created_at);
                 CREATE TABLE IF NOT EXISTS run_snapshots (
                    run_id TEXT NOT NULL REFERENCES runs(id),
                    inventory_id TEXT NOT NULL REFERENCES inventories(id),
                    snapshot_id TEXT NOT NULL REFERENCES source_snapshots(id),
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (run_id, inventory_id)
                 );
                 PRAGMA user_version = 2;
                 COMMIT;",
        )?;
        Ok(())
    }

    fn migrate_v3(connection: &Connection) -> Result<(), StoreError> {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS raw_graphs (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES projects(id),
                snapshot_id TEXT NOT NULL REFERENCES source_snapshots(id),
                policy_hash TEXT NOT NULL,
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS raw_graphs_project_idx
                ON raw_graphs(project_id, created_at);
             CREATE TABLE IF NOT EXISTS run_graphs (
                run_id TEXT NOT NULL REFERENCES runs(id),
                graph_id TEXT NOT NULL REFERENCES raw_graphs(id),
                created_at TEXT NOT NULL,
                PRIMARY KEY (run_id, graph_id)
             );
             CREATE TABLE IF NOT EXISTS audit_reports (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES projects(id),
                graph_id TEXT NOT NULL REFERENCES raw_graphs(id),
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS audit_reports_project_idx
                ON audit_reports(project_id, created_at);
             CREATE TABLE IF NOT EXISTS run_reports (
                run_id TEXT NOT NULL REFERENCES runs(id),
                report_id TEXT NOT NULL REFERENCES audit_reports(id),
                created_at TEXT NOT NULL,
                PRIMARY KEY (run_id, report_id)
             );
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
        Ok(())
    }

    fn migrate_v4(connection: &Connection) -> Result<(), StoreError> {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS adapter_conformance_reports (
                id TEXT PRIMARY KEY,
                package_hash TEXT NOT NULL,
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS adapters (
                package_hash TEXT PRIMARY KEY,
                adapter_id TEXT NOT NULL,
                version TEXT NOT NULL,
                source_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                conformance_report_id TEXT NOT NULL REFERENCES adapter_conformance_reports(id),
                installed_at TEXT NOT NULL,
                UNIQUE(adapter_id, version)
             );
             CREATE INDEX IF NOT EXISTS adapters_identity_idx
                ON adapters(adapter_id, version);
             CREATE TABLE IF NOT EXISTS adapter_detections (
                id TEXT PRIMARY KEY,
                graph_id TEXT NOT NULL REFERENCES raw_graphs(id),
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS run_adapter_detections (
                run_id TEXT NOT NULL REFERENCES runs(id),
                detection_id TEXT NOT NULL REFERENCES adapter_detections(id),
                created_at TEXT NOT NULL,
                PRIMARY KEY(run_id, detection_id)
             );
             CREATE TABLE IF NOT EXISTS adapter_plans (
                id TEXT PRIMARY KEY,
                detection_id TEXT NOT NULL REFERENCES adapter_detections(id),
                graph_id TEXT NOT NULL REFERENCES raw_graphs(id),
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS run_adapter_plans (
                run_id TEXT NOT NULL REFERENCES runs(id),
                plan_id TEXT NOT NULL REFERENCES adapter_plans(id),
                created_at TEXT NOT NULL,
                PRIMARY KEY(run_id, plan_id)
             );
             CREATE TABLE IF NOT EXISTS semantic_graphs (
                id TEXT PRIMARY KEY,
                raw_graph_id TEXT NOT NULL REFERENCES raw_graphs(id),
                adapter_plan_id TEXT NOT NULL REFERENCES adapter_plans(id),
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS run_semantic_graphs (
                run_id TEXT NOT NULL REFERENCES runs(id),
                semantic_graph_id TEXT NOT NULL REFERENCES semantic_graphs(id),
                created_at TEXT NOT NULL,
                PRIMARY KEY(run_id, semantic_graph_id)
             );
             CREATE TABLE IF NOT EXISTS semantic_audit_reports (
                id TEXT PRIMARY KEY,
                semantic_graph_id TEXT NOT NULL REFERENCES semantic_graphs(id),
                adapter_plan_id TEXT NOT NULL REFERENCES adapter_plans(id),
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS run_semantic_reports (
                run_id TEXT NOT NULL REFERENCES runs(id),
                semantic_report_id TEXT NOT NULL REFERENCES semantic_audit_reports(id),
                created_at TEXT NOT NULL,
                PRIMARY KEY(run_id, semantic_report_id)
             );
             CREATE TABLE IF NOT EXISTS projection_catalogs (
                id TEXT PRIMARY KEY,
                semantic_graph_id TEXT NOT NULL REFERENCES semantic_graphs(id),
                semantic_report_id TEXT NOT NULL REFERENCES semantic_audit_reports(id),
                adapter_plan_id TEXT NOT NULL REFERENCES adapter_plans(id),
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS run_projection_catalogs (
                run_id TEXT NOT NULL REFERENCES runs(id),
                catalog_id TEXT NOT NULL REFERENCES projection_catalogs(id),
                created_at TEXT NOT NULL,
                PRIMARY KEY(run_id, catalog_id)
             );
             PRAGMA user_version = 4;
             COMMIT;",
        )?;
        Ok(())
    }

    fn migrate_v5(connection: &Connection) -> Result<(), StoreError> {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS source_acquisitions (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES projects(id),
                manifest_hash TEXT NOT NULL REFERENCES artifacts(sha256),
                created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS source_acquisitions_project_idx
                ON source_acquisitions(project_id, created_at);
             CREATE TABLE IF NOT EXISTS run_acquisitions (
                run_id TEXT PRIMARY KEY REFERENCES runs(id),
                acquisition_id TEXT NOT NULL REFERENCES source_acquisitions(id),
                created_at TEXT NOT NULL
             );
             PRAGMA user_version = 5;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Register a project. The source path is recorded but not modified.
    pub fn create_project(
        &self,
        id: ProjectId,
        display_name: impl Into<String>,
        source_path: PathBuf,
    ) -> Result<Project, StoreError> {
        let project = Project {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id,
            display_name: display_name.into(),
            source_path,
            created_at: Utc::now(),
        };
        let connection = self.connection()?;
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO projects(id, display_name, source_path, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                project.id.to_string(),
                project.display_name,
                project.source_path.to_string_lossy(),
                timestamp(project.created_at)
            ],
        )?;
        if inserted == 0 {
            return Err(StoreError::ProjectExists(project.id.to_string()));
        }
        Ok(project)
    }

    /// Load one project.
    pub fn get_project(&self, id: &ProjectId) -> Result<Project, StoreError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, display_name, source_path, created_at FROM projects WHERE id = ?1",
                [id.to_string()],
                project_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "project",
                id: id.to_string(),
            })
    }

    /// List projects in stable ID order.
    pub fn list_projects(&self) -> Result<Vec<Project>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, display_name, source_path, created_at FROM projects ORDER BY id",
        )?;
        let rows = statement.query_map([], project_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Persist immutable provenance for a project whose source is platform managed.
    pub fn calculate_source_acquisition_id(
        acquisition: &SourceAcquisition,
    ) -> Result<Sha256Digest, StoreError> {
        Ok(hash_value(&SourceAcquisitionIdentity {
            schema_version: &acquisition.schema_version,
            project_id: &acquisition.project_id,
            kind: acquisition.kind,
            source_locator: &acquisition.source_locator,
            repository_id: acquisition.repository_id,
            requested_revision: acquisition.requested_revision.as_deref(),
            resolved_revision: acquisition.resolved_revision.as_deref(),
            retrieval_url: acquisition.retrieval_url.as_deref(),
            provider_api_version: acquisition.provider_api_version.as_deref(),
            content_sha256: &acquisition.content_sha256,
            total_bytes: acquisition.total_bytes,
        })?)
    }

    /// Persist immutable provenance for a project whose source is platform managed.
    pub fn save_source_acquisition(
        &self,
        acquisition: &SourceAcquisition,
    ) -> Result<SourceAcquisition, StoreError> {
        let project = self.get_project(&acquisition.project_id)?;
        let managed_path = acquisition.managed_path.canonicalize()?;
        let project_path = project.source_path.canonicalize()?;
        let imports_root = self.root.join("imports").canonicalize()?;
        if project_path != managed_path || !managed_path.starts_with(&imports_root) {
            return Err(StoreError::InvalidRelationship(format!(
                "source acquisition path {} is not the project's managed import path",
                acquisition.managed_path.display()
            )));
        }
        match acquisition.kind {
            SourceAcquisitionKind::UploadedFolder if !managed_path.is_dir() => {
                return Err(StoreError::InvalidRelationship(
                    "uploaded-folder acquisition is not a directory".to_owned(),
                ));
            }
            SourceAcquisitionKind::UploadedArchive | SourceAcquisitionKind::PublicGithub
                if !managed_path.is_file() =>
            {
                return Err(StoreError::InvalidRelationship(
                    "archive acquisition is not a regular file".to_owned(),
                ));
            }
            _ => {}
        }
        if matches!(
            acquisition.kind,
            SourceAcquisitionKind::UploadedArchive | SourceAcquisitionKind::PublicGithub
        ) {
            let (observed_hash, observed_bytes) = sha256_reader(File::open(&managed_path)?)?;
            if observed_hash != acquisition.content_sha256
                || observed_bytes != acquisition.total_bytes
            {
                return Err(StoreError::InvalidRelationship(format!(
                    "managed archive bytes do not match acquisition {} provenance",
                    acquisition.id
                )));
            }
        }
        let expected_id = Self::calculate_source_acquisition_id(acquisition)?;
        if expected_id != acquisition.id {
            return Err(StoreError::InvalidRelationship(format!(
                "source acquisition identity is {}, expected {expected_id}",
                acquisition.id
            )));
        }
        let artifact = self.put_json_manifest(
            acquisition,
            format!("source-acquisition-{}.json", acquisition.id),
        )?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO source_acquisitions(id, project_id, manifest_hash, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                acquisition.id.to_string(),
                acquisition.project_id.to_string(),
                artifact.sha256.to_string(),
                timestamp(acquisition.created_at),
            ],
        )?;
        self.get_source_acquisition(&acquisition.id)
    }

    /// Load and integrity-check one immutable source-acquisition record.
    pub fn get_source_acquisition(
        &self,
        acquisition_id: &Sha256Digest,
    ) -> Result<SourceAcquisition, StoreError> {
        let connection = self.connection()?;
        let manifest_hash: String = connection
            .query_row(
                "SELECT manifest_hash FROM source_acquisitions WHERE id = ?1",
                [acquisition_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "source acquisition",
                id: acquisition_id.to_string(),
            })?;
        let bytes = self.read_artifact_bounded(&Sha256Digest::new(manifest_hash)?, 1024 * 1024)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// List managed source acquisitions, optionally restricted to one project.
    pub fn list_source_acquisitions(
        &self,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<SourceAcquisition>, StoreError> {
        let connection = self.connection()?;
        let ids = if let Some(project_id) = project_id {
            let mut statement = connection.prepare(
                "SELECT id FROM source_acquisitions WHERE project_id = ?1 ORDER BY created_at, id",
            )?;
            statement
                .query_map([project_id.to_string()], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            let mut statement = connection.prepare(
                "SELECT id FROM source_acquisitions ORDER BY project_id, created_at, id",
            )?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        ids.into_iter()
            .map(|id| {
                Sha256Digest::new(id)
                    .map_err(StoreError::from)
                    .and_then(|id| self.get_source_acquisition(&id))
            })
            .collect()
    }

    /// Pin one run to the exact immutable source acquisition it audits.
    pub fn bind_run_acquisition(
        &self,
        run_id: RunId,
        acquisition_id: &Sha256Digest,
    ) -> Result<SourceAcquisition, StoreError> {
        let run = self.get_run(run_id)?;
        let acquisition = self.get_source_acquisition(acquisition_id)?;
        if run.project_id != acquisition.project_id {
            return Err(StoreError::InvalidRelationship(format!(
                "run {run_id} and acquisition {acquisition_id} belong to different projects"
            )));
        }
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO run_acquisitions(run_id, acquisition_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![
                run_id.to_string(),
                acquisition_id.to_string(),
                timestamp(Utc::now()),
            ],
        )?;
        Ok(acquisition)
    }

    /// Load the exact immutable source acquisition pinned to a run.
    pub fn get_run_acquisition(&self, run_id: RunId) -> Result<SourceAcquisition, StoreError> {
        let connection = self.connection()?;
        let acquisition_id: String = connection
            .query_row(
                "SELECT acquisition_id FROM run_acquisitions WHERE run_id = ?1",
                [run_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "run acquisition binding",
                id: run_id.to_string(),
            })?;
        self.get_source_acquisition(&Sha256Digest::new(acquisition_id)?)
    }

    /// Start a durable run and append its first event atomically.
    pub fn create_run(&self, project_id: ProjectId, mode: RunMode) -> Result<Run, StoreError> {
        self.get_project(&project_id)?;
        let now = Utc::now();
        let run = Run {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id: RunId::new(),
            project_id,
            mode,
            state: RunState::Running,
            created_at: now,
            updated_at: now,
        };
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO runs(id, project_id, mode, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run.id.to_string(),
                run.project_id.to_string(),
                run.mode.to_string(),
                run.state.to_string(),
                timestamp(run.created_at),
                timestamp(run.updated_at)
            ],
        )?;
        append_event_tx(
            &transaction,
            run.id,
            "run_created",
            None,
            &json!({"mode": run.mode, "project_id": run.project_id}),
        )?;
        transaction.commit()?;
        Ok(run)
    }

    /// Load one run.
    pub fn get_run(&self, id: RunId) -> Result<Run, StoreError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, project_id, mode, state, created_at, updated_at
                 FROM runs WHERE id = ?1",
                [id.to_string()],
                run_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "run",
                id: id.to_string(),
            })
    }

    /// List all runs, optionally restricted to one project.
    pub fn list_runs(&self, project_id: Option<&ProjectId>) -> Result<Vec<Run>, StoreError> {
        let connection = self.connection()?;
        if let Some(project_id) = project_id {
            let mut statement = connection.prepare(
                "SELECT id, project_id, mode, state, created_at, updated_at
                 FROM runs WHERE project_id = ?1 ORDER BY created_at, id",
            )?;
            let rows = statement.query_map([project_id.to_string()], run_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(StoreError::from)
        } else {
            let mut statement = connection.prepare(
                "SELECT id, project_id, mode, state, created_at, updated_at
                 FROM runs ORDER BY created_at, id",
            )?;
            let rows = statement.query_map([], run_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(StoreError::from)
        }
    }

    /// Record a new current output for a stage and require a fresh approval.
    pub fn record_stage_output(
        &self,
        run_id: RunId,
        stage_id: StageId,
        output_hash: Sha256Digest,
    ) -> Result<StageOutput, StoreError> {
        self.get_run(run_id)?;
        let now = Utc::now();
        let output = StageOutput {
            run_id,
            stage_id,
            output_hash,
            state: StageState::WaitingApproval,
            updated_at: now,
        };
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing: Option<(String, String, String)> = transaction
            .query_row(
                "SELECT output_hash, state, updated_at FROM stage_outputs
                 WHERE run_id = ?1 AND stage_id = ?2",
                params![run_id.to_string(), output.stage_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((existing_hash, existing_state, existing_updated_at)) = existing {
            let existing_hash = Sha256Digest::from_str(&existing_hash)?;
            if existing_hash == output.output_hash {
                return Ok(StageOutput {
                    run_id,
                    stage_id: output.stage_id,
                    output_hash: existing_hash,
                    state: StageState::from_str(&existing_state)?,
                    updated_at: parse_timestamp(&existing_updated_at)?,
                });
            }
        }
        transaction.execute(
            "INSERT INTO stage_outputs(run_id, stage_id, output_hash, state, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(run_id, stage_id) DO UPDATE SET
                output_hash = excluded.output_hash,
                state = excluded.state,
                updated_at = excluded.updated_at",
            params![
                output.run_id.to_string(),
                output.stage_id.to_string(),
                output.output_hash.to_string(),
                output.state.to_string(),
                timestamp(output.updated_at)
            ],
        )?;
        transaction.execute(
            "UPDATE runs SET state = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                RunState::WaitingApproval.to_string(),
                timestamp(now),
                run_id.to_string()
            ],
        )?;
        append_event_tx(
            &transaction,
            run_id,
            "stage_output_recorded",
            Some(&output.stage_id),
            &json!({"output_hash": output.output_hash}),
        )?;
        transaction.commit()?;
        Ok(output)
    }

    /// Load the latest output for a stage.
    pub fn get_stage_output(
        &self,
        run_id: RunId,
        stage_id: &StageId,
    ) -> Result<StageOutput, StoreError> {
        self.find_stage_output(run_id, stage_id)?
            .ok_or_else(|| StoreError::NotFound {
                kind: "stage output",
                id: format!("{run_id}/{stage_id}"),
            })
    }

    /// Find the latest output for a stage without treating absence as an error.
    pub fn find_stage_output(
        &self,
        run_id: RunId,
        stage_id: &StageId,
    ) -> Result<Option<StageOutput>, StoreError> {
        let connection = self.connection()?;
        Ok(connection
            .query_row(
                "SELECT run_id, stage_id, output_hash, state, updated_at
                 FROM stage_outputs WHERE run_id = ?1 AND stage_id = ?2",
                params![run_id.to_string(), stage_id.to_string()],
                stage_output_from_row,
            )
            .optional()?)
    }

    /// Require a stage to be approved for an exact output hash.
    pub fn require_approved_stage(
        &self,
        run_id: RunId,
        stage_id: &StageId,
        expected_hash: &Sha256Digest,
    ) -> Result<(), StoreError> {
        match self.get_stage_output(run_id, stage_id) {
            Ok(output)
                if output.state == StageState::Approved && output.output_hash == *expected_hash =>
            {
                Ok(())
            }
            Ok(output) => Err(StoreError::RequiredApprovalMissing {
                stage_id: stage_id.clone(),
                expected_hash: expected_hash.clone(),
                actual_state: format!("{} at {}", output.state, output.output_hash),
            }),
            Err(StoreError::NotFound { .. }) => Err(StoreError::RequiredApprovalMissing {
                stage_id: stage_id.clone(),
                expected_hash: expected_hash.clone(),
                actual_state: "missing".to_owned(),
            }),
            Err(error) => Err(error),
        }
    }

    /// Record a manual decision only if the expected hash is still current.
    pub fn decide_stage(&self, request: ApprovalRequest) -> Result<Approval, StoreError> {
        if matches!(
            request.decision,
            Decision::Reject | Decision::RequestChanges
        ) && request
            .reason
            .as_deref()
            .is_none_or(|reason| reason.trim().is_empty())
        {
            return Err(StoreError::ReasonRequired(request.decision));
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        require_reviewable_stage(&transaction, &request)?;

        let now = Utc::now();
        let approval = Approval {
            id: ApprovalId::new(),
            run_id: request.run_id,
            stage_id: request.stage_id,
            stage_output_hash: request.expected_output_hash,
            decision: request.decision,
            actor_id: request.actor_id,
            reason: request.reason,
            created_at: now,
        };
        transaction.execute(
            "INSERT INTO approvals(
                id, run_id, stage_id, stage_output_hash, decision, actor_id, reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                approval.id.to_string(),
                approval.run_id.to_string(),
                approval.stage_id.to_string(),
                approval.stage_output_hash.to_string(),
                approval.decision.to_string(),
                approval.actor_id.to_string(),
                approval.reason,
                timestamp(approval.created_at)
            ],
        )?;
        let (stage_state, run_state) = match approval.decision {
            Decision::Approve => (StageState::Approved, RunState::Running),
            Decision::Reject => (StageState::Rejected, RunState::Rejected),
            Decision::RequestChanges => (StageState::NeedsChanges, RunState::NeedsChanges),
        };
        transaction.execute(
            "UPDATE stage_outputs SET state = ?1, updated_at = ?2
             WHERE run_id = ?3 AND stage_id = ?4",
            params![
                stage_state.to_string(),
                timestamp(now),
                approval.run_id.to_string(),
                approval.stage_id.to_string()
            ],
        )?;
        transaction.execute(
            "UPDATE runs SET state = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                run_state.to_string(),
                timestamp(now),
                approval.run_id.to_string()
            ],
        )?;
        append_event_tx(
            &transaction,
            approval.run_id,
            "approval_recorded",
            Some(&approval.stage_id),
            &json!({
                "approval_id": approval.id.to_string(),
                "actor_id": approval.actor_id,
                "decision": approval.decision,
                "output_hash": approval.stage_output_hash,
                "reason": approval.reason,
            }),
        )?;
        transaction.commit()?;
        Ok(approval)
    }

    /// Mark a run complete after approval of an exact terminal-stage output.
    pub fn complete_run(
        &self,
        run_id: RunId,
        final_stage: &StageId,
        final_output_hash: &Sha256Digest,
    ) -> Result<Run, StoreError> {
        self.require_approved_stage(run_id, final_stage, final_output_hash)?;
        let run = self.get_run(run_id)?;
        if run.state == RunState::Complete {
            return Ok(run);
        }
        if run.state != RunState::Running {
            return Err(StoreError::InvalidRelationship(format!(
                "run {run_id} is {}, not eligible for completion",
                run.state
            )));
        }
        let now = Utc::now();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE runs SET state = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                RunState::Complete.to_string(),
                timestamp(now),
                run_id.to_string(),
            ],
        )?;
        append_event_tx(
            &transaction,
            run_id,
            "run_completed",
            Some(final_stage),
            &json!({"final_output_hash": final_output_hash}),
        )?;
        transaction.commit()?;
        self.get_run(run_id)
    }

    /// List decisions for a run in creation order.
    pub fn list_approvals(&self, run_id: RunId) -> Result<Vec<Approval>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, run_id, stage_id, stage_output_hash, decision, actor_id, reason, created_at
             FROM approvals WHERE run_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([run_id.to_string()], approval_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// List append-only events for a run.
    pub fn list_events(&self, run_id: RunId) -> Result<Vec<RunEvent>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT event_id, run_id, sequence, kind, stage_id, payload_json,
                    created_at, previous_hash, event_hash
             FROM run_events WHERE run_id = ?1 ORDER BY sequence",
        )?;
        let rows = statement.query_map([run_id.to_string()], event_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Verify sequence, previous-hash links, and event hashes for a run.
    pub fn verify_event_chain(&self, run_id: RunId) -> Result<(), StoreError> {
        let events = self.list_events(run_id)?;
        let mut previous: Option<Sha256Digest> = None;
        for (index, event) in events.iter().enumerate() {
            let expected_sequence =
                u64::try_from(index + 1).map_err(|_| StoreError::NumericRange {
                    field: "event sequence",
                    value: u64::MAX,
                })?;
            if event.sequence != expected_sequence {
                return Err(StoreError::EventChainInvalid {
                    run_id,
                    sequence: event.sequence,
                    reason: format!("expected sequence {expected_sequence}"),
                });
            }
            if event.previous_hash != previous {
                return Err(StoreError::EventChainInvalid {
                    run_id,
                    sequence: event.sequence,
                    reason: "previous hash does not match preceding event".to_owned(),
                });
            }
            let created_at = timestamp(event.created_at);
            let identity = EventIdentity {
                event_id: event.id.to_string(),
                run_id: event.run_id.to_string(),
                sequence: event.sequence,
                kind: &event.kind,
                stage_id: event.stage_id.as_ref().map(StageId::as_str),
                payload: &event.payload,
                created_at: &created_at,
                previous_hash: event.previous_hash.as_ref().map(Sha256Digest::as_str),
            };
            let calculated = hash_value(&identity)?;
            if calculated != event.event_hash {
                return Err(StoreError::EventChainInvalid {
                    run_id,
                    sequence: event.sequence,
                    reason: "event hash does not match canonical event".to_owned(),
                });
            }
            previous = Some(event.event_hash.clone());
        }
        Ok(())
    }

    /// Store bytes by content identity, deduplicating identical content.
    pub fn put_bytes(
        &self,
        bytes: &[u8],
        original_name: Option<String>,
        media_type: Option<String>,
    ) -> Result<ArtifactRecord, StoreError> {
        let temporary_path = self.temporary_root.join(Uuid::new_v4().to_string());
        {
            let mut file = File::create(&temporary_path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        self.commit_temporary_artifact(
            &temporary_path,
            original_name,
            media_type,
            Some(bytes.len() as u64),
        )
    }

    /// Canonically serialize JSON and store it by the identity of its exact bytes.
    pub fn put_json<T: Serialize>(
        &self,
        value: &T,
        original_name: Option<String>,
    ) -> Result<ArtifactRecord, StoreError> {
        self.put_bytes(
            &canonical_json(value)?,
            original_name,
            Some("application/json".to_owned()),
        )
    }

    /// Stream a file into the content-addressed store.
    pub fn put_file(
        &self,
        source: &Path,
        media_type: Option<String>,
    ) -> Result<ArtifactRecord, StoreError> {
        let original_name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        let input = BufReader::new(File::open(source)?);
        self.put_reader(input, original_name, media_type)
    }

    /// Stream arbitrary content into the content-addressed store.
    pub fn put_reader(
        &self,
        mut input: impl Read,
        original_name: Option<String>,
        media_type: Option<String>,
    ) -> Result<ArtifactRecord, StoreError> {
        let temporary_path = self.temporary_root.join(Uuid::new_v4().to_string());
        let mut output = BufWriter::new(File::create(&temporary_path)?);
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            hasher.update(&buffer[..count]);
            size = size.saturating_add(count as u64);
        }
        output.flush()?;
        output.get_ref().sync_all()?;
        drop(output);
        let digest = Sha256Digest::new(format!("{:x}", hasher.finalize()))?;
        self.commit_known_artifact(&temporary_path, digest, size, original_name, media_type)
    }

    fn commit_temporary_artifact(
        &self,
        temporary_path: &Path,
        original_name: Option<String>,
        media_type: Option<String>,
        known_size: Option<u64>,
    ) -> Result<ArtifactRecord, StoreError> {
        let mut input = BufReader::new(File::open(temporary_path)?);
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            size = size.saturating_add(count as u64);
        }
        if let Some(known_size) = known_size {
            debug_assert_eq!(known_size, size);
        }
        let digest = Sha256Digest::new(format!("{:x}", hasher.finalize()))?;
        self.commit_known_artifact(temporary_path, digest, size, original_name, media_type)
    }

    fn commit_known_artifact(
        &self,
        temporary_path: &Path,
        digest: Sha256Digest,
        size: u64,
        original_name: Option<String>,
        media_type: Option<String>,
    ) -> Result<ArtifactRecord, StoreError> {
        let relative_path = artifact_relative_path(&digest);
        let destination = self.root.join(&relative_path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        if destination.exists() {
            fs::remove_file(temporary_path)?;
        } else {
            fs::rename(temporary_path, &destination)?;
        }

        let size_i64 = i64::try_from(size).map_err(|_| StoreError::NumericRange {
            field: "artifact size",
            value: size,
        })?;
        let now = Utc::now();
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO artifacts(
                sha256, size_bytes, storage_path, original_name, media_type, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                digest.to_string(),
                size_i64,
                relative_path.to_string_lossy(),
                original_name,
                media_type,
                timestamp(now)
            ],
        )?;
        self.get_artifact(&digest)
    }

    /// Load artifact metadata.
    pub fn get_artifact(&self, digest: &Sha256Digest) -> Result<ArtifactRecord, StoreError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT sha256, size_bytes, storage_path, original_name, media_type, created_at
                 FROM artifacts WHERE sha256 = ?1",
                [digest.to_string()],
                artifact_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "artifact",
                id: digest.to_string(),
            })
    }

    /// Resolve a stored artifact to its local content path.
    pub fn artifact_path(&self, digest: &Sha256Digest) -> Result<PathBuf, StoreError> {
        let record = self.get_artifact(digest)?;
        let expected = artifact_relative_path(digest);
        if record.storage_path != expected {
            return Err(StoreError::InvalidRelationship(format!(
                "artifact {digest} storage path is not content-derived"
            )));
        }
        Ok(self.root.join(expected))
    }

    /// Read and verify an artifact's complete bytes.
    pub fn read_artifact(&self, digest: &Sha256Digest) -> Result<Vec<u8>, StoreError> {
        let path = self.artifact_path(digest)?;
        let bytes = fs::read(path)?;
        let calculated = sha256_bytes(&bytes)?;
        if calculated != *digest {
            return Err(StoreError::ArtifactIntegrity {
                expected: digest.clone(),
                calculated,
            });
        }
        Ok(bytes)
    }

    /// Read and verify an artifact while never buffering more than `maximum_bytes + 1`.
    pub fn read_artifact_bounded(
        &self,
        digest: &Sha256Digest,
        maximum_bytes: u64,
    ) -> Result<Vec<u8>, StoreError> {
        let record = self.get_artifact(digest)?;
        if record.size_bytes > maximum_bytes {
            return Err(StoreError::ArtifactReadLimit {
                digest: digest.clone(),
                observed_bytes: record.size_bytes,
                maximum_bytes,
            });
        }
        let path = self.artifact_path(digest)?;
        let observed_bytes = fs::metadata(&path)?.len();
        if observed_bytes > maximum_bytes {
            return Err(StoreError::ArtifactReadLimit {
                digest: digest.clone(),
                observed_bytes,
                maximum_bytes,
            });
        }
        let capacity = usize::try_from(observed_bytes).unwrap_or(usize::MAX);
        let mut bytes = Vec::with_capacity(capacity);
        let sentinel_limit = maximum_bytes.saturating_add(1);
        BufReader::new(File::open(path)?)
            .take(sentinel_limit)
            .read_to_end(&mut bytes)?;
        let buffered_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if buffered_bytes > maximum_bytes {
            return Err(StoreError::ArtifactReadLimit {
                digest: digest.clone(),
                observed_bytes: buffered_bytes,
                maximum_bytes,
            });
        }
        let calculated = sha256_bytes(&bytes)?;
        if calculated != *digest {
            return Err(StoreError::ArtifactIntegrity {
                expected: digest.clone(),
                calculated,
            });
        }
        Ok(bytes)
    }

    /// Read, integrity-check, and deserialize a JSON artifact within a hard byte limit.
    pub fn read_json_bounded<T: DeserializeOwned>(
        &self,
        digest: &Sha256Digest,
        maximum_bytes: u64,
    ) -> Result<T, StoreError> {
        let bytes = self.read_artifact_bounded(digest, maximum_bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Stream and verify an artifact without loading the complete file into memory.
    pub fn verify_artifact(&self, digest: &Sha256Digest) -> Result<(), StoreError> {
        let path = self.artifact_path(digest)?;
        let mut input = BufReader::new(File::open(path)?);
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let calculated = Sha256Digest::new(format!("{:x}", hasher.finalize()))?;
        if calculated != *digest {
            return Err(StoreError::ArtifactIntegrity {
                expected: digest.clone(),
                calculated,
            });
        }
        Ok(())
    }

    /// Persist a complete inventory manifest as a content-addressed artifact.
    pub fn save_inventory(&self, inventory: &Inventory) -> Result<Inventory, StoreError> {
        let connection = self.connection()?;
        let existing_id: Option<String> = connection
            .query_row(
                "SELECT id FROM inventories WHERE run_id = ?1 AND inventory_hash = ?2",
                params![
                    inventory.run_id.to_string(),
                    inventory.inventory_hash.to_string()
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_id) = existing_id {
            return self.get_inventory(InventoryId::from_str(&existing_id)?);
        }
        drop(connection);
        let manifest = canonical_json(inventory)?;
        let artifact = self.put_bytes(
            &manifest,
            Some(format!("inventory-{}.json", inventory.id)),
            Some("application/json".to_owned()),
        )?;
        let connection = self.connection()?;
        let inserted = connection.execute(
            "INSERT OR IGNORE INTO inventories(
                id, run_id, project_id, inventory_hash, policy_hash,
                manifest_hash, source_kind, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                inventory.id.to_string(),
                inventory.run_id.to_string(),
                inventory.project_id.to_string(),
                inventory.inventory_hash.to_string(),
                inventory.policy_hash.to_string(),
                artifact.sha256.to_string(),
                inventory.source_kind.to_string(),
                timestamp(inventory.created_at),
            ],
        )?;
        if inserted == 0 {
            let existing_id: String = connection.query_row(
                "SELECT id FROM inventories WHERE run_id = ?1 AND inventory_hash = ?2",
                params![
                    inventory.run_id.to_string(),
                    inventory.inventory_hash.to_string()
                ],
                |row| row.get(0),
            )?;
            return self.get_inventory(InventoryId::from_str(&existing_id)?);
        }
        self.get_inventory(inventory.id)
    }

    /// Load and integrity-check an inventory manifest.
    pub fn get_inventory(&self, id: InventoryId) -> Result<Inventory, StoreError> {
        let connection = self.connection()?;
        let manifest_hash: String = connection
            .query_row(
                "SELECT manifest_hash FROM inventories WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "inventory",
                id: id.to_string(),
            })?;
        let bytes = self.read_artifact(&Sha256Digest::new(manifest_hash)?)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// List inventory manifests for a run.
    pub fn list_inventories(&self, run_id: RunId) -> Result<Vec<Inventory>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT id FROM inventories WHERE run_id = ?1 ORDER BY created_at, id")?;
        let ids = statement
            .query_map([run_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                InventoryId::from_str(&id)
                    .map_err(StoreError::from)
                    .and_then(|id| self.get_inventory(id))
            })
            .collect()
    }

    /// Persist an immutable source snapshot manifest.
    pub fn save_snapshot(
        &self,
        snapshot: &SourceSnapshot,
        run_id: RunId,
        inventory_id: InventoryId,
    ) -> Result<SourceSnapshot, StoreError> {
        let connection = self.connection()?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM source_snapshots WHERE id = ?1",
                [snapshot.id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        drop(connection);
        if exists {
            let mut connection = self.connection()?;
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT OR IGNORE INTO run_snapshots(run_id, inventory_id, snapshot_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    run_id.to_string(),
                    inventory_id.to_string(),
                    snapshot.id.to_string(),
                    timestamp(Utc::now()),
                ],
            )?;
            transaction.commit()?;
            return self.get_snapshot(&snapshot.id);
        }
        let manifest = canonical_json(snapshot)?;
        let artifact = self.put_bytes(
            &manifest,
            Some(format!("snapshot-{}.json", snapshot.id)),
            Some("application/json".to_owned()),
        )?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO source_snapshots(
                id, project_id, inventory_hash, policy_hash, manifest_hash, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.id.to_string(),
                snapshot.project_id.to_string(),
                snapshot.inventory_hash.to_string(),
                snapshot.policy_hash.to_string(),
                artifact.sha256.to_string(),
                timestamp(snapshot.created_at),
            ],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO run_snapshots(run_id, inventory_id, snapshot_id, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                run_id.to_string(),
                inventory_id.to_string(),
                snapshot.id.to_string(),
                timestamp(Utc::now()),
            ],
        )?;
        transaction.commit()?;
        self.get_snapshot(&snapshot.id)
    }

    /// Load and integrity-check a source snapshot manifest.
    pub fn get_snapshot(&self, id: &Sha256Digest) -> Result<SourceSnapshot, StoreError> {
        let connection = self.connection()?;
        let manifest_hash: String = connection
            .query_row(
                "SELECT manifest_hash FROM source_snapshots WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "snapshot",
                id: id.to_string(),
            })?;
        let bytes = self.read_artifact(&Sha256Digest::new(manifest_hash)?)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// List source snapshots for a run.
    pub fn list_snapshots(&self, run_id: RunId) -> Result<Vec<SourceSnapshot>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT snapshot_id FROM run_snapshots
             WHERE run_id = ?1 ORDER BY created_at, snapshot_id",
        )?;
        let ids = statement
            .query_map([run_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                Sha256Digest::new(id)
                    .map_err(StoreError::from)
                    .and_then(|id| self.get_snapshot(&id))
            })
            .collect()
    }

    /// Require a snapshot to be attached to the specified run.
    pub fn require_run_snapshot(
        &self,
        run_id: RunId,
        snapshot_id: &Sha256Digest,
    ) -> Result<(), StoreError> {
        let connection = self.connection()?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM run_snapshots WHERE run_id = ?1 AND snapshot_id = ?2 LIMIT 1",
                params![run_id.to_string(), snapshot_id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(StoreError::NotFound {
                kind: "run snapshot binding",
                id: format!("{run_id}/{snapshot_id}"),
            })
        }
    }

    /// Persist a raw project graph and attach it to a run.
    pub fn save_graph(
        &self,
        graph: &RawProjectGraph,
        run_id: RunId,
    ) -> Result<RawProjectGraph, StoreError> {
        self.require_run_snapshot(run_id, &graph.snapshot_id)?;
        let connection = self.connection()?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM raw_graphs WHERE id = ?1",
                [graph.id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        drop(connection);
        if !exists {
            let manifest = canonical_json(graph)?;
            let artifact = self.put_bytes(
                &manifest,
                Some(format!("raw-graph-{}.json", graph.id)),
                Some("application/json".to_owned()),
            )?;
            let connection = self.connection()?;
            connection.execute(
                "INSERT OR IGNORE INTO raw_graphs(
                    id, project_id, snapshot_id, policy_hash, manifest_hash, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    graph.id.to_string(),
                    graph.project_id.to_string(),
                    graph.snapshot_id.to_string(),
                    graph.policy_hash.to_string(),
                    artifact.sha256.to_string(),
                    timestamp(graph.created_at),
                ],
            )?;
        }
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO run_graphs(run_id, graph_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![
                run_id.to_string(),
                graph.id.to_string(),
                timestamp(Utc::now()),
            ],
        )?;
        self.get_graph(&graph.id)
    }

    /// Load and integrity-check a raw project graph manifest.
    pub fn get_graph(&self, id: &Sha256Digest) -> Result<RawProjectGraph, StoreError> {
        let connection = self.connection()?;
        let manifest_hash: String = connection
            .query_row(
                "SELECT manifest_hash FROM raw_graphs WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "raw graph",
                id: id.to_string(),
            })?;
        let bytes = self.read_artifact(&Sha256Digest::new(manifest_hash)?)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// List raw project graphs attached to a run.
    pub fn list_graphs(&self, run_id: RunId) -> Result<Vec<RawProjectGraph>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT graph_id FROM run_graphs WHERE run_id = ?1 ORDER BY created_at, graph_id",
        )?;
        let ids = statement
            .query_map([run_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                Sha256Digest::new(id)
                    .map_err(StoreError::from)
                    .and_then(|id| self.get_graph(&id))
            })
            .collect()
    }

    /// Require a raw graph to be attached to the specified run.
    pub fn require_run_graph(
        &self,
        run_id: RunId,
        graph_id: &Sha256Digest,
    ) -> Result<(), StoreError> {
        let connection = self.connection()?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM run_graphs WHERE run_id = ?1 AND graph_id = ?2 LIMIT 1",
                params![run_id.to_string(), graph_id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(StoreError::NotFound {
                kind: "run graph binding",
                id: format!("{run_id}/{graph_id}"),
            })
        }
    }

    /// Persist a generic audit report and attach it to a run.
    pub fn save_report(
        &self,
        report: &AuditReport,
        run_id: RunId,
    ) -> Result<AuditReport, StoreError> {
        self.require_run_graph(run_id, &report.graph_id)?;
        let connection = self.connection()?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM audit_reports WHERE id = ?1",
                [report.id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        drop(connection);
        if !exists {
            let manifest = canonical_json(report)?;
            let artifact = self.put_bytes(
                &manifest,
                Some(format!("audit-report-{}.json", report.id)),
                Some("application/json".to_owned()),
            )?;
            let connection = self.connection()?;
            connection.execute(
                "INSERT OR IGNORE INTO audit_reports(
                    id, project_id, graph_id, manifest_hash, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    report.id.to_string(),
                    report.project_id.to_string(),
                    report.graph_id.to_string(),
                    artifact.sha256.to_string(),
                    timestamp(report.created_at),
                ],
            )?;
        }
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO run_reports(run_id, report_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![
                run_id.to_string(),
                report.id.to_string(),
                timestamp(Utc::now()),
            ],
        )?;
        self.get_report(&report.id)
    }

    /// Load and integrity-check a generic audit report manifest.
    pub fn get_report(&self, id: &Sha256Digest) -> Result<AuditReport, StoreError> {
        let connection = self.connection()?;
        let manifest_hash: String = connection
            .query_row(
                "SELECT manifest_hash FROM audit_reports WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "audit report",
                id: id.to_string(),
            })?;
        let bytes = self.read_artifact(&Sha256Digest::new(manifest_hash)?)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// List generic audit reports attached to a run.
    pub fn list_reports(&self, run_id: RunId) -> Result<Vec<AuditReport>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT report_id FROM run_reports WHERE run_id = ?1 ORDER BY created_at, report_id",
        )?;
        let ids = statement
            .query_map([run_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                Sha256Digest::new(id)
                    .map_err(StoreError::from)
                    .and_then(|id| self.get_report(&id))
            })
            .collect()
    }

    /// Persist one digest-specific adapter conformance report.
    pub fn save_adapter_conformance(
        &self,
        report: &AdapterConformanceReport,
    ) -> Result<AdapterConformanceReport, StoreError> {
        let artifact =
            self.put_json_manifest(report, format!("adapter-conformance-{}.json", report.id))?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO adapter_conformance_reports(
                id, package_hash, manifest_hash, created_at
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                report.id.to_string(),
                report.package_hash.to_string(),
                artifact.sha256.to_string(),
                timestamp(report.created_at),
            ],
        )?;
        self.get_adapter_conformance(&report.id)
    }

    /// Load and integrity-check an adapter conformance report.
    pub fn get_adapter_conformance(
        &self,
        id: &Sha256Digest,
    ) -> Result<AdapterConformanceReport, StoreError> {
        self.load_json_manifest(
            "SELECT manifest_hash FROM adapter_conformance_reports WHERE id = ?1",
            id,
            "adapter conformance report",
        )
    }

    /// Install one conformant immutable declarative adapter package.
    pub fn save_adapter(&self, record: &AdapterRecord) -> Result<AdapterRecord, StoreError> {
        let calculated = hash_value(&record.package)?;
        if calculated != record.package_hash {
            return Err(StoreError::InvalidRelationship(format!(
                "adapter package declares {}, calculated {calculated}",
                record.package_hash
            )));
        }
        self.get_artifact(&record.source_hash)?;
        let conformance = self.get_adapter_conformance(&record.conformance_report_id)?;
        if !conformance.passed || conformance.package_hash != record.package_hash {
            return Err(StoreError::InvalidRelationship(
                "adapter package is not bound to a passing conformance report".to_owned(),
            ));
        }
        let connection = self.connection()?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT package_hash FROM adapters WHERE adapter_id = ?1 AND version = ?2",
                params![
                    record.package.manifest.id.to_string(),
                    record.package.manifest.version
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing_hash = Sha256Digest::new(existing)?;
            if existing_hash == record.package_hash {
                let mut installed = self.get_adapter(&existing_hash)?;
                if installed.conformance_report_id == record.conformance_report_id {
                    return Ok(installed);
                }

                // The package and its original source provenance remain immutable, but a
                // newly versioned conformance suite must be able to certify the same
                // canonical package again. Rotate only the certificate pointer and the
                // record manifest; the old report and manifest remain in the CAS.
                installed.conformance_report_id = record.conformance_report_id.clone();
                let artifact = self.put_json_manifest(
                    &installed,
                    format!("adapter-package-{}.json", installed.package_hash),
                )?;
                connection.execute(
                    "UPDATE adapters
                     SET manifest_hash = ?1, conformance_report_id = ?2
                     WHERE package_hash = ?3",
                    params![
                        artifact.sha256.to_string(),
                        installed.conformance_report_id.to_string(),
                        installed.package_hash.to_string(),
                    ],
                )?;
                return self.get_adapter(&existing_hash);
            }
            return Err(StoreError::AdapterVersionConflict {
                adapter_id: record.package.manifest.id.to_string(),
                version: record.package.manifest.version.clone(),
                existing_hash,
                new_hash: record.package_hash.clone(),
            });
        }
        drop(connection);
        let artifact = self.put_json_manifest(
            record,
            format!("adapter-package-{}.json", record.package_hash),
        )?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO adapters(
                package_hash, adapter_id, version, source_hash, manifest_hash,
                conformance_report_id, installed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.package_hash.to_string(),
                record.package.manifest.id.to_string(),
                record.package.manifest.version,
                record.source_hash.to_string(),
                artifact.sha256.to_string(),
                record.conformance_report_id.to_string(),
                timestamp(record.installed_at),
            ],
        )?;
        self.get_adapter(&record.package_hash)
    }

    /// Load one installed adapter by authoritative package hash.
    pub fn get_adapter(&self, package_hash: &Sha256Digest) -> Result<AdapterRecord, StoreError> {
        self.load_json_manifest(
            "SELECT manifest_hash FROM adapters WHERE package_hash = ?1",
            package_hash,
            "adapter package",
        )
    }

    /// List installed adapters deterministically with the platform's safe default bound.
    pub fn list_adapters(&self) -> Result<Vec<AdapterRecord>, StoreError> {
        self.list_adapters_bounded(256)
    }

    /// List installed adapters after a database-only cardinality preflight.
    ///
    /// The count is checked before any package manifest is read or deserialized, so a
    /// registry larger than the host-selected interpreter limit cannot force an
    /// unbounded allocation.
    pub fn list_adapters_bounded(&self, maximum: usize) -> Result<Vec<AdapterRecord>, StoreError> {
        let connection = self.connection()?;
        let observed_i64: i64 =
            connection.query_row("SELECT COUNT(*) FROM adapters", [], |row| row.get(0))?;
        let observed = u64::try_from(observed_i64).map_err(|_| StoreError::NumericRange {
            field: "adapter registry count",
            value: 0,
        })?;
        let maximum = u64::try_from(maximum).unwrap_or(u64::MAX);
        if observed > maximum {
            return Err(StoreError::RecordReadLimit {
                kind: "adapter registry",
                observed,
                maximum,
            });
        }
        let mut statement = connection.prepare(
            "SELECT package_hash FROM adapters ORDER BY adapter_id, version, package_hash",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                Sha256Digest::new(id)
                    .map_err(StoreError::from)
                    .and_then(|id| self.get_adapter(&id))
            })
            .collect()
    }

    /// Persist and attach a deterministic adapter detection report.
    pub fn save_adapter_detection(
        &self,
        run_id: RunId,
        report: &AdapterDetectionReport,
    ) -> Result<AdapterDetectionReport, StoreError> {
        self.require_run_graph(run_id, &report.graph_id)?;
        let artifact =
            self.put_json_manifest(report, format!("adapter-detection-{}.json", report.id))?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO adapter_detections(id, graph_id, manifest_hash, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                report.id.to_string(),
                report.graph_id.to_string(),
                artifact.sha256.to_string(),
                timestamp(report.created_at),
            ],
        )?;
        self.attach_run_record(
            "INSERT OR IGNORE INTO run_adapter_detections(run_id, detection_id, created_at)
             VALUES (?1, ?2, ?3)",
            run_id,
            &report.id,
        )?;
        self.get_adapter_detection(&report.id)
    }

    /// Load one adapter detection report.
    pub fn get_adapter_detection(
        &self,
        id: &Sha256Digest,
    ) -> Result<AdapterDetectionReport, StoreError> {
        self.load_json_manifest(
            "SELECT manifest_hash FROM adapter_detections WHERE id = ?1",
            id,
            "adapter detection",
        )
    }

    /// List adapter detections attached to a run.
    pub fn list_adapter_detections(
        &self,
        run_id: RunId,
    ) -> Result<Vec<AdapterDetectionReport>, StoreError> {
        let ids = self.list_run_content_ids(
            "SELECT detection_id FROM run_adapter_detections
             WHERE run_id = ?1 ORDER BY created_at, detection_id",
            run_id,
        )?;
        ids.into_iter()
            .map(|id| self.get_adapter_detection(&id))
            .collect()
    }

    /// Require an adapter detection report to be attached to the specified run.
    pub fn require_run_adapter_detection(
        &self,
        run_id: RunId,
        detection_id: &Sha256Digest,
    ) -> Result<(), StoreError> {
        self.require_run_content(
            "SELECT 1 FROM run_adapter_detections WHERE run_id = ?1 AND detection_id = ?2",
            run_id,
            detection_id,
            "run adapter detection binding",
        )
    }

    /// Persist and attach an immutable selected adapter plan.
    pub fn save_adapter_plan(
        &self,
        run_id: RunId,
        plan: &AdapterPlan,
    ) -> Result<AdapterPlan, StoreError> {
        self.require_run_content(
            "SELECT 1 FROM run_adapter_detections WHERE run_id = ?1 AND detection_id = ?2",
            run_id,
            &plan.detection_report_id,
            "run adapter detection binding",
        )?;
        self.require_run_graph(run_id, &plan.raw_graph_id)?;
        let detection = self.get_adapter_detection(&plan.detection_report_id)?;
        if detection.graph_id != plan.raw_graph_id {
            return Err(StoreError::InvalidRelationship(
                "adapter plan graph differs from its detection graph".to_owned(),
            ));
        }
        if plan.adapters.is_empty() {
            return Err(StoreError::InvalidRelationship(
                "adapter plan must select at least one eligible package".to_owned(),
            ));
        }
        let mut selected_adapter_ids = std::collections::BTreeSet::new();
        for locked in &plan.adapters {
            if !selected_adapter_ids.insert(locked.adapter_id.clone()) {
                return Err(StoreError::InvalidRelationship(format!(
                    "adapter plan selects {} more than once",
                    locked.adapter_id
                )));
            }
            let detected = detection
                .candidates
                .iter()
                .find(|candidate| candidate.package_hash == locked.package_hash)
                .ok_or_else(|| {
                    StoreError::InvalidRelationship(format!(
                        "adapter package {} does not appear in detection {}",
                        locked.package_hash, detection.id
                    ))
                })?;
            if !detected.eligible {
                return Err(StoreError::InvalidRelationship(format!(
                    "adapter package {} was not eligible in detection {}",
                    locked.package_hash, detection.id
                )));
            }
            if detected.adapter_id != locked.adapter_id
                || detected.adapter_version != locked.adapter_version
            {
                return Err(StoreError::InvalidRelationship(format!(
                    "adapter lock {} identity differs from detection {}",
                    locked.package_hash, detection.id
                )));
            }
            let record = self.get_adapter(&locked.package_hash)?;
            if record.package.manifest.id != locked.adapter_id
                || record.package.manifest.version != locked.adapter_version
                || record.conformance_report_id != locked.conformance_report_id
                || record.package.manifest.capabilities != locked.capabilities
            {
                return Err(StoreError::InvalidRelationship(format!(
                    "adapter lock {} does not match installed package {}",
                    locked.adapter_id, locked.package_hash
                )));
            }
        }
        let artifact = self.put_json_manifest(plan, format!("adapter-plan-{}.json", plan.id))?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO adapter_plans(
                id, detection_id, graph_id, manifest_hash, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                plan.id.to_string(),
                plan.detection_report_id.to_string(),
                plan.raw_graph_id.to_string(),
                artifact.sha256.to_string(),
                timestamp(plan.created_at),
            ],
        )?;
        self.attach_run_record(
            "INSERT OR IGNORE INTO run_adapter_plans(run_id, plan_id, created_at)
             VALUES (?1, ?2, ?3)",
            run_id,
            &plan.id,
        )?;
        self.get_adapter_plan(&plan.id)
    }

    /// Load one immutable adapter plan.
    pub fn get_adapter_plan(&self, id: &Sha256Digest) -> Result<AdapterPlan, StoreError> {
        self.load_json_manifest(
            "SELECT manifest_hash FROM adapter_plans WHERE id = ?1",
            id,
            "adapter plan",
        )
    }

    /// List adapter plans attached to a run.
    pub fn list_adapter_plans(&self, run_id: RunId) -> Result<Vec<AdapterPlan>, StoreError> {
        let ids = self.list_run_content_ids(
            "SELECT plan_id FROM run_adapter_plans
             WHERE run_id = ?1 ORDER BY created_at, plan_id",
            run_id,
        )?;
        ids.into_iter()
            .map(|id| self.get_adapter_plan(&id))
            .collect()
    }

    /// Require an adapter plan to be attached to the specified run.
    pub fn require_run_adapter_plan(
        &self,
        run_id: RunId,
        plan_id: &Sha256Digest,
    ) -> Result<(), StoreError> {
        self.require_run_content(
            "SELECT 1 FROM run_adapter_plans WHERE run_id = ?1 AND plan_id = ?2",
            run_id,
            plan_id,
            "run adapter plan binding",
        )
    }

    /// Persist and attach an adapter-produced semantic graph.
    pub fn save_semantic_graph(
        &self,
        run_id: RunId,
        graph: &SemanticGraph,
    ) -> Result<SemanticGraph, StoreError> {
        self.require_run_content(
            "SELECT 1 FROM run_adapter_plans WHERE run_id = ?1 AND plan_id = ?2",
            run_id,
            &graph.adapter_plan_id,
            "run adapter plan binding",
        )?;
        self.require_run_graph(run_id, &graph.raw_graph_id)?;
        let plan = self.get_adapter_plan(&graph.adapter_plan_id)?;
        if plan.raw_graph_id != graph.raw_graph_id || plan.adapters != graph.adapters {
            return Err(StoreError::InvalidRelationship(
                "semantic graph does not match its adapter plan".to_owned(),
            ));
        }
        let artifact =
            self.put_json_manifest(graph, format!("semantic-graph-{}.json", graph.id))?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO semantic_graphs(
                id, raw_graph_id, adapter_plan_id, manifest_hash, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                graph.id.to_string(),
                graph.raw_graph_id.to_string(),
                graph.adapter_plan_id.to_string(),
                artifact.sha256.to_string(),
                timestamp(graph.created_at),
            ],
        )?;
        self.attach_run_record(
            "INSERT OR IGNORE INTO run_semantic_graphs(
                run_id, semantic_graph_id, created_at
             ) VALUES (?1, ?2, ?3)",
            run_id,
            &graph.id,
        )?;
        self.get_semantic_graph(&graph.id)
    }

    /// Load one semantic graph.
    pub fn get_semantic_graph(&self, id: &Sha256Digest) -> Result<SemanticGraph, StoreError> {
        self.load_json_manifest(
            "SELECT manifest_hash FROM semantic_graphs WHERE id = ?1",
            id,
            "semantic graph",
        )
    }

    /// List semantic graphs attached to a run.
    pub fn list_semantic_graphs(&self, run_id: RunId) -> Result<Vec<SemanticGraph>, StoreError> {
        let ids = self.list_run_content_ids(
            "SELECT semantic_graph_id FROM run_semantic_graphs
             WHERE run_id = ?1 ORDER BY created_at, semantic_graph_id",
            run_id,
        )?;
        ids.into_iter()
            .map(|id| self.get_semantic_graph(&id))
            .collect()
    }

    /// Require a semantic graph to be attached to the specified run.
    pub fn require_run_semantic_graph(
        &self,
        run_id: RunId,
        graph_id: &Sha256Digest,
    ) -> Result<(), StoreError> {
        self.require_run_content(
            "SELECT 1 FROM run_semantic_graphs
             WHERE run_id = ?1 AND semantic_graph_id = ?2",
            run_id,
            graph_id,
            "run semantic graph binding",
        )
    }

    /// Persist and attach a semantic adapter audit report.
    pub fn save_semantic_report(
        &self,
        run_id: RunId,
        report: &SemanticAuditReport,
    ) -> Result<SemanticAuditReport, StoreError> {
        self.require_run_content(
            "SELECT 1 FROM run_semantic_graphs
             WHERE run_id = ?1 AND semantic_graph_id = ?2",
            run_id,
            &report.semantic_graph_id,
            "run semantic graph binding",
        )?;
        let graph = self.get_semantic_graph(&report.semantic_graph_id)?;
        if graph.adapter_plan_id != report.adapter_plan_id {
            return Err(StoreError::InvalidRelationship(
                "semantic report plan differs from its graph plan".to_owned(),
            ));
        }
        let artifact =
            self.put_json_manifest(report, format!("semantic-report-{}.json", report.id))?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO semantic_audit_reports(
                id, semantic_graph_id, adapter_plan_id, manifest_hash, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                report.id.to_string(),
                report.semantic_graph_id.to_string(),
                report.adapter_plan_id.to_string(),
                artifact.sha256.to_string(),
                timestamp(report.created_at),
            ],
        )?;
        self.attach_run_record(
            "INSERT OR IGNORE INTO run_semantic_reports(
                run_id, semantic_report_id, created_at
             ) VALUES (?1, ?2, ?3)",
            run_id,
            &report.id,
        )?;
        self.get_semantic_report(&report.id)
    }

    /// Load one semantic adapter report.
    pub fn get_semantic_report(
        &self,
        id: &Sha256Digest,
    ) -> Result<SemanticAuditReport, StoreError> {
        self.load_json_manifest(
            "SELECT manifest_hash FROM semantic_audit_reports WHERE id = ?1",
            id,
            "semantic audit report",
        )
    }

    /// List semantic adapter reports attached to a run.
    pub fn list_semantic_reports(
        &self,
        run_id: RunId,
    ) -> Result<Vec<SemanticAuditReport>, StoreError> {
        let ids = self.list_run_content_ids(
            "SELECT semantic_report_id FROM run_semantic_reports
             WHERE run_id = ?1 ORDER BY created_at, semantic_report_id",
            run_id,
        )?;
        ids.into_iter()
            .map(|id| self.get_semantic_report(&id))
            .collect()
    }

    /// Require a semantic audit report to be attached to the specified run.
    pub fn require_run_semantic_report(
        &self,
        run_id: RunId,
        report_id: &Sha256Digest,
    ) -> Result<(), StoreError> {
        self.require_run_content(
            "SELECT 1 FROM run_semantic_reports
             WHERE run_id = ?1 AND semantic_report_id = ?2",
            run_id,
            report_id,
            "run semantic report binding",
        )
    }

    /// Persist and attach a resolved semantic projection catalog.
    pub fn save_projection_catalog(
        &self,
        run_id: RunId,
        catalog: &ProjectionCatalog,
    ) -> Result<ProjectionCatalog, StoreError> {
        self.require_run_content(
            "SELECT 1 FROM run_semantic_graphs
             WHERE run_id = ?1 AND semantic_graph_id = ?2",
            run_id,
            &catalog.semantic_graph_id,
            "run semantic graph binding",
        )?;
        self.require_run_content(
            "SELECT 1 FROM run_semantic_reports
             WHERE run_id = ?1 AND semantic_report_id = ?2",
            run_id,
            &catalog.semantic_report_id,
            "run semantic report binding",
        )?;
        let graph = self.get_semantic_graph(&catalog.semantic_graph_id)?;
        let report = self.get_semantic_report(&catalog.semantic_report_id)?;
        if graph.adapter_plan_id != catalog.adapter_plan_id
            || report.adapter_plan_id != catalog.adapter_plan_id
            || report.semantic_graph_id != catalog.semantic_graph_id
        {
            return Err(StoreError::InvalidRelationship(
                "projection catalog predecessors do not share one plan/graph".to_owned(),
            ));
        }
        let artifact =
            self.put_json_manifest(catalog, format!("projection-catalog-{}.json", catalog.id))?;
        let connection = self.connection()?;
        connection.execute(
            "INSERT OR IGNORE INTO projection_catalogs(
                id, semantic_graph_id, semantic_report_id, adapter_plan_id,
                manifest_hash, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                catalog.id.to_string(),
                catalog.semantic_graph_id.to_string(),
                catalog.semantic_report_id.to_string(),
                catalog.adapter_plan_id.to_string(),
                artifact.sha256.to_string(),
                timestamp(catalog.created_at),
            ],
        )?;
        self.attach_run_record(
            "INSERT OR IGNORE INTO run_projection_catalogs(run_id, catalog_id, created_at)
             VALUES (?1, ?2, ?3)",
            run_id,
            &catalog.id,
        )?;
        self.get_projection_catalog(&catalog.id)
    }

    /// Load one semantic projection catalog.
    pub fn get_projection_catalog(
        &self,
        id: &Sha256Digest,
    ) -> Result<ProjectionCatalog, StoreError> {
        self.load_json_manifest(
            "SELECT manifest_hash FROM projection_catalogs WHERE id = ?1",
            id,
            "projection catalog",
        )
    }

    /// List semantic projection catalogs attached to a run.
    pub fn list_projection_catalogs(
        &self,
        run_id: RunId,
    ) -> Result<Vec<ProjectionCatalog>, StoreError> {
        let ids = self.list_run_content_ids(
            "SELECT catalog_id FROM run_projection_catalogs
             WHERE run_id = ?1 ORDER BY created_at, catalog_id",
            run_id,
        )?;
        ids.into_iter()
            .map(|id| self.get_projection_catalog(&id))
            .collect()
    }

    fn put_json_manifest(
        &self,
        value: &impl Serialize,
        name: String,
    ) -> Result<ArtifactRecord, StoreError> {
        self.put_json(value, Some(name))
    }

    fn load_json_manifest<T: DeserializeOwned>(
        &self,
        sql: &'static str,
        id: &Sha256Digest,
        kind: &'static str,
    ) -> Result<T, StoreError> {
        let connection = self.connection()?;
        let manifest_hash: String = connection
            .query_row(sql, [id.to_string()], |row| row.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind,
                id: id.to_string(),
            })?;
        let bytes = self.read_artifact(&Sha256Digest::new(manifest_hash)?)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn attach_run_record(
        &self,
        sql: &'static str,
        run_id: RunId,
        id: &Sha256Digest,
    ) -> Result<(), StoreError> {
        let connection = self.connection()?;
        connection.execute(
            sql,
            params![run_id.to_string(), id.to_string(), timestamp(Utc::now())],
        )?;
        Ok(())
    }

    fn require_run_content(
        &self,
        sql: &'static str,
        run_id: RunId,
        id: &Sha256Digest,
        kind: &'static str,
    ) -> Result<(), StoreError> {
        let connection = self.connection()?;
        let exists = connection
            .query_row(sql, params![run_id.to_string(), id.to_string()], |_| Ok(()))
            .optional()?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(StoreError::NotFound {
                kind,
                id: format!("{run_id}/{id}"),
            })
        }
    }

    fn list_run_content_ids(
        &self,
        sql: &'static str,
        run_id: RunId,
    ) -> Result<Vec<Sha256Digest>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(sql)?;
        let ids = statement
            .query_map([run_id.to_string()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(Sha256Digest::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

fn append_event_tx(
    transaction: &Transaction<'_>,
    run_id: RunId,
    kind: &str,
    stage_id: Option<&StageId>,
    payload: &Value,
) -> Result<RunEvent, StoreError> {
    let previous: Option<(i64, String)> = transaction
        .query_row(
            "SELECT sequence, event_hash FROM run_events
             WHERE run_id = ?1 ORDER BY sequence DESC LIMIT 1",
            [run_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (sequence, previous_hash) = match previous {
        Some((sequence, hash)) => {
            let next = sequence.checked_add(1).ok_or(StoreError::NumericRange {
                field: "event sequence",
                value: u64::MAX,
            })?;
            (
                u64::try_from(next).map_err(|_| StoreError::NumericRange {
                    field: "event sequence",
                    value: u64::MAX,
                })?,
                Some(Sha256Digest::new(hash)?),
            )
        }
        None => (1, None),
    };
    let id = EventId::new();
    let created_at = Utc::now();
    let created_at_text = timestamp(created_at);
    let identity = EventIdentity {
        event_id: id.to_string(),
        run_id: run_id.to_string(),
        sequence,
        kind,
        stage_id: stage_id.map(StageId::as_str),
        payload,
        created_at: &created_at_text,
        previous_hash: previous_hash.as_ref().map(Sha256Digest::as_str),
    };
    let event_hash = hash_value(&identity)?;
    let sequence_i64 = i64::try_from(sequence).map_err(|_| StoreError::NumericRange {
        field: "event sequence",
        value: sequence,
    })?;
    transaction.execute(
        "INSERT INTO run_events(
            run_id, sequence, event_id, kind, stage_id, payload_json,
            created_at, previous_hash, event_hash
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            run_id.to_string(),
            sequence_i64,
            id.to_string(),
            kind,
            stage_id.map(ToString::to_string),
            serde_json::to_string(payload)?,
            created_at_text,
            previous_hash.as_ref().map(ToString::to_string),
            event_hash.to_string()
        ],
    )?;
    Ok(RunEvent {
        id,
        run_id,
        sequence,
        kind: kind.to_owned(),
        stage_id: stage_id.cloned(),
        payload: payload.clone(),
        created_at,
        previous_hash,
        event_hash,
    })
}

fn require_reviewable_stage(
    transaction: &Transaction<'_>,
    request: &ApprovalRequest,
) -> Result<(), StoreError> {
    let (current_hash, current_state): (String, String) = transaction
        .query_row(
            "SELECT output_hash, state FROM stage_outputs WHERE run_id = ?1 AND stage_id = ?2",
            params![request.run_id.to_string(), request.stage_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            kind: "stage output",
            id: format!("{}/{}", request.run_id, request.stage_id),
        })?;
    let current_hash = Sha256Digest::from_str(&current_hash)?;
    if current_hash != request.expected_output_hash {
        return Err(StoreError::StaleApproval {
            expected: request.expected_output_hash.clone(),
            current: current_hash,
        });
    }
    let current_state = StageState::from_str(&current_state)?;
    if current_state != StageState::WaitingApproval {
        return Err(StoreError::StageNotAwaitingApproval {
            stage_id: request.stage_id.clone(),
            state: current_state,
        });
    }
    Ok(())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, StoreError> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn artifact_relative_path(digest: &Sha256Digest) -> PathBuf {
    let value = digest.as_str();
    PathBuf::from("artifacts")
        .join("sha256")
        .join(&value[0..2])
        .join(&value[2..4])
        .join(value)
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let id: String = row.get(0)?;
    let created_at: String = row.get(3)?;
    Ok(Project {
        schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
        id: ProjectId::from_str(&id).map_err(to_sql_conversion_error)?,
        display_name: row.get(1)?,
        source_path: PathBuf::from(row.get::<_, String>(2)?),
        created_at: parse_timestamp(&created_at).map_err(to_sql_conversion_error)?,
    })
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    let id: String = row.get(0)?;
    let project_id: String = row.get(1)?;
    let mode: String = row.get(2)?;
    let state: String = row.get(3)?;
    let created_at: String = row.get(4)?;
    let updated_at: String = row.get(5)?;
    Ok(Run {
        schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
        id: RunId::from_str(&id).map_err(to_sql_conversion_error)?,
        project_id: ProjectId::from_str(&project_id).map_err(to_sql_conversion_error)?,
        mode: RunMode::from_str(&mode).map_err(to_sql_conversion_error)?,
        state: RunState::from_str(&state).map_err(to_sql_conversion_error)?,
        created_at: parse_timestamp(&created_at).map_err(to_sql_conversion_error)?,
        updated_at: parse_timestamp(&updated_at).map_err(to_sql_conversion_error)?,
    })
}

fn stage_output_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StageOutput> {
    let run_id: String = row.get(0)?;
    let stage_id: String = row.get(1)?;
    let output_hash: String = row.get(2)?;
    let state: String = row.get(3)?;
    let updated_at: String = row.get(4)?;
    Ok(StageOutput {
        run_id: RunId::from_str(&run_id).map_err(to_sql_conversion_error)?,
        stage_id: StageId::from_str(&stage_id).map_err(to_sql_conversion_error)?,
        output_hash: Sha256Digest::from_str(&output_hash).map_err(to_sql_conversion_error)?,
        state: StageState::from_str(&state).map_err(to_sql_conversion_error)?,
        updated_at: parse_timestamp(&updated_at).map_err(to_sql_conversion_error)?,
    })
}

fn approval_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Approval> {
    let id: String = row.get(0)?;
    let run_id: String = row.get(1)?;
    let stage_id: String = row.get(2)?;
    let stage_output_hash: String = row.get(3)?;
    let decision: String = row.get(4)?;
    let actor_id: String = row.get(5)?;
    let created_at: String = row.get(7)?;
    Ok(Approval {
        id: ApprovalId::from_str(&id).map_err(to_sql_conversion_error)?,
        run_id: RunId::from_str(&run_id).map_err(to_sql_conversion_error)?,
        stage_id: StageId::from_str(&stage_id).map_err(to_sql_conversion_error)?,
        stage_output_hash: Sha256Digest::from_str(&stage_output_hash)
            .map_err(to_sql_conversion_error)?,
        decision: Decision::from_str(&decision).map_err(to_sql_conversion_error)?,
        actor_id: ActorId::from_str(&actor_id).map_err(to_sql_conversion_error)?,
        reason: row.get(6)?,
        created_at: parse_timestamp(&created_at).map_err(to_sql_conversion_error)?,
    })
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunEvent> {
    let event_id: String = row.get(0)?;
    let run_id: String = row.get(1)?;
    let sequence_i64: i64 = row.get(2)?;
    let stage_id: Option<String> = row.get(4)?;
    let payload: String = row.get(5)?;
    let created_at: String = row.get(6)?;
    let previous_hash: Option<String> = row.get(7)?;
    let event_hash: String = row.get(8)?;
    let sequence = u64::try_from(sequence_i64).map_err(to_sql_conversion_error)?;
    Ok(RunEvent {
        id: EventId::from_str(&event_id).map_err(to_sql_conversion_error)?,
        run_id: RunId::from_str(&run_id).map_err(to_sql_conversion_error)?,
        sequence,
        kind: row.get(3)?,
        stage_id: stage_id
            .map(|value| StageId::from_str(&value))
            .transpose()
            .map_err(to_sql_conversion_error)?,
        payload: serde_json::from_str(&payload).map_err(to_sql_conversion_error)?,
        created_at: parse_timestamp(&created_at).map_err(to_sql_conversion_error)?,
        previous_hash: previous_hash
            .map(Sha256Digest::new)
            .transpose()
            .map_err(to_sql_conversion_error)?,
        event_hash: Sha256Digest::new(event_hash).map_err(to_sql_conversion_error)?,
    })
}

fn artifact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactRecord> {
    let digest: String = row.get(0)?;
    let size_i64: i64 = row.get(1)?;
    let created_at: String = row.get(5)?;
    let size_bytes = u64::try_from(size_i64).map_err(to_sql_conversion_error)?;
    Ok(ArtifactRecord {
        sha256: Sha256Digest::new(digest).map_err(to_sql_conversion_error)?,
        size_bytes,
        storage_path: PathBuf::from(row.get::<_, String>(2)?),
        original_name: row.get(3)?,
        media_type: row.get(4)?,
        created_at: parse_timestamp(&created_at).map_err(to_sql_conversion_error)?,
    })
}

fn to_sql_conversion_error(
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use qbm_domain::{
        ADAPTER_API_VERSION, ADAPTER_PACKAGE_SCHEMA_VERSION, AdapterCandidate, AdapterCapability,
        AdapterFixture, AdapterId, AdapterLock, AdapterManifest, AdapterPackage, IntakeSourceKind,
        MappingPack, ProjectionDefinition, RawProjectGraph, ResolvedProjection, SemanticPolicyPack,
        SourceAcquisitionKind,
    };
    use tempfile::TempDir;

    fn setup() -> (TempDir, PlatformStore, ProjectId) {
        let directory = TempDir::new().unwrap();
        let store = PlatformStore::open(directory.path()).unwrap();
        let project_id = ProjectId::new("demo").unwrap();
        store
            .create_project(project_id.clone(), "Demo", directory.path().join("source"))
            .unwrap();
        (directory, store, project_id)
    }

    fn digest(label: &str) -> Sha256Digest {
        sha256_bytes(label.as_bytes()).unwrap()
    }

    fn adapter_package(id: &str, version: &str, description: &str) -> AdapterPackage {
        AdapterPackage {
            schema_version: ADAPTER_PACKAGE_SCHEMA_VERSION.to_owned(),
            manifest: AdapterManifest {
                schema_version: "qbm.adapter-manifest/v1".to_owned(),
                adapter_api_version: ADAPTER_API_VERSION.to_owned(),
                id: AdapterId::new(id).unwrap(),
                version: version.to_owned(),
                display_name: format!("{id} adapter"),
                description: description.to_owned(),
                minimum_platform_schema: DOMAIN_SCHEMA_VERSION.to_owned(),
                capabilities: [
                    AdapterCapability::RawGraphRead,
                    AdapterCapability::EmitSemanticNodes,
                    AdapterCapability::EmitFindings,
                    AdapterCapability::EmitProjections,
                ]
                .into_iter()
                .collect(),
                license: "Apache-2.0".to_owned(),
            },
            detection: Vec::new(),
            mappings: MappingPack {
                schema_version: "qbm.mapping-pack/v1".to_owned(),
                nodes: Vec::new(),
                edges: Vec::new(),
            },
            policies: SemanticPolicyPack {
                schema_version: "qbm.semantic-policy-pack/v1".to_owned(),
                rules: Vec::new(),
            },
            projections: Vec::<ProjectionDefinition>::new(),
            fixtures: Vec::<AdapterFixture>::new(),
        }
    }

    fn conformance_report(
        package_hash: Sha256Digest,
        passed: bool,
        label: &str,
    ) -> AdapterConformanceReport {
        AdapterConformanceReport {
            schema_version: "qbm.adapter-conformance/v1".to_owned(),
            id: digest(&format!("conformance:{label}:{package_hash}:{passed}")),
            package_hash,
            passed,
            checks: 1,
            issues: Vec::new(),
            created_at: Utc::now(),
        }
    }

    fn install_adapter(
        store: &PlatformStore,
        package: AdapterPackage,
        source_label: &str,
    ) -> AdapterRecord {
        let package_hash = hash_value(&package).unwrap();
        let source = store
            .put_bytes(
                source_label.as_bytes(),
                Some(format!("{source_label}.json")),
                Some("application/json".to_owned()),
            )
            .unwrap();
        let report = conformance_report(package_hash.clone(), true, source_label);
        store.save_adapter_conformance(&report).unwrap();
        let record = AdapterRecord {
            package_hash,
            source_hash: source.sha256,
            conformance_report_id: report.id,
            package,
            installed_at: Utc::now(),
        };
        store.save_adapter(&record).unwrap()
    }

    fn attach_empty_graph(
        store: &PlatformStore,
        project_id: &ProjectId,
        run_id: RunId,
        label: &str,
    ) -> RawProjectGraph {
        let inventory = Inventory {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id: InventoryId::new(),
            run_id,
            project_id: project_id.clone(),
            source_kind: IntakeSourceKind::Directory,
            source_path: store.root().join(label),
            source_container_hash: None,
            policy_hash: digest(&format!("inventory-policy:{label}")),
            inventory_hash: digest(&format!("inventory:{label}")),
            entries: Vec::new(),
            included_files: 0,
            excluded_entries: 0,
            blocked_entries: 0,
            total_included_bytes: 0,
            created_at: Utc::now(),
        };
        store.save_inventory(&inventory).unwrap();
        let snapshot = SourceSnapshot {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id: digest(&format!("snapshot:{label}")),
            inventory_hash: inventory.inventory_hash.clone(),
            policy_hash: inventory.policy_hash.clone(),
            project_id: project_id.clone(),
            artifacts: Vec::new(),
            total_bytes: 0,
            created_at: Utc::now(),
        };
        store
            .save_snapshot(&snapshot, run_id, inventory.id)
            .unwrap();
        let graph = RawProjectGraph {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id: digest(&format!("raw-graph:{label}")),
            snapshot_id: snapshot.id,
            project_id: project_id.clone(),
            policy_hash: digest(&format!("graph-policy:{label}")),
            nodes: Vec::new(),
            edges: Vec::new(),
            diagnostics: Vec::new(),
            created_at: Utc::now(),
        };
        store.save_graph(&graph, run_id).unwrap()
    }

    fn candidate(record: &AdapterRecord, eligible: bool) -> AdapterCandidate {
        AdapterCandidate {
            adapter_id: record.package.manifest.id.clone(),
            adapter_version: record.package.manifest.version.clone(),
            package_hash: record.package_hash.clone(),
            eligible,
            confidence_bps: if eligible { 10_000 } else { 0 },
            matched_weight: u64::from(eligible),
            maximum_weight: 1,
            rules: Vec::new(),
        }
    }

    fn adapter_lock(record: &AdapterRecord) -> AdapterLock {
        AdapterLock {
            adapter_id: record.package.manifest.id.clone(),
            adapter_version: record.package.manifest.version.clone(),
            package_hash: record.package_hash.clone(),
            conformance_report_id: record.conformance_report_id.clone(),
            capabilities: record.package.manifest.capabilities.clone(),
        }
    }

    struct AdapterLineage {
        detection: AdapterDetectionReport,
        plan: AdapterPlan,
        graph: SemanticGraph,
        report: SemanticAuditReport,
        catalog: ProjectionCatalog,
    }

    fn persist_adapter_lineage(
        store: &PlatformStore,
        run_id: RunId,
        raw_graph: &RawProjectGraph,
        record: &AdapterRecord,
        label: &str,
    ) -> AdapterLineage {
        let detection = AdapterDetectionReport {
            schema_version: "qbm.adapter-detection/v1".to_owned(),
            id: digest(&format!("detection:{label}")),
            graph_id: raw_graph.id.clone(),
            candidates: vec![candidate(record, true)],
            created_at: Utc::now(),
        };
        store.save_adapter_detection(run_id, &detection).unwrap();
        let lock = adapter_lock(record);
        let plan = AdapterPlan {
            schema_version: "qbm.adapter-plan/v1".to_owned(),
            id: digest(&format!("adapter-plan:{label}")),
            detection_report_id: detection.id.clone(),
            raw_graph_id: raw_graph.id.clone(),
            adapters: vec![lock.clone()],
            configuration_hash: digest(&format!("adapter-config:{label}")),
            created_at: Utc::now(),
        };
        store.save_adapter_plan(run_id, &plan).unwrap();
        let graph = SemanticGraph {
            schema_version: "qbm.semantic-graph/v1".to_owned(),
            id: digest(&format!("semantic-graph:{label}")),
            raw_graph_id: raw_graph.id.clone(),
            adapter_plan_id: plan.id.clone(),
            adapters: vec![lock],
            nodes: Vec::new(),
            edges: Vec::new(),
            created_at: Utc::now(),
        };
        store.save_semantic_graph(run_id, &graph).unwrap();
        let report = SemanticAuditReport {
            schema_version: "qbm.semantic-audit/v1".to_owned(),
            id: digest(&format!("semantic-report:{label}")),
            semantic_graph_id: graph.id.clone(),
            adapter_plan_id: plan.id.clone(),
            findings: Vec::new(),
            summary: qbm_domain::AuditSummary {
                total: 0,
                info: 0,
                warnings: 0,
                errors: 0,
                critical: 0,
            },
            created_at: Utc::now(),
        };
        store.save_semantic_report(run_id, &report).unwrap();
        let catalog = ProjectionCatalog {
            schema_version: "qbm.projection-catalog/v1".to_owned(),
            id: digest(&format!("projection-catalog:{label}")),
            semantic_graph_id: graph.id.clone(),
            semantic_report_id: report.id.clone(),
            adapter_plan_id: plan.id.clone(),
            projections: Vec::<ResolvedProjection>::new(),
            created_at: Utc::now(),
        };
        store.save_projection_catalog(run_id, &catalog).unwrap();
        AdapterLineage {
            detection,
            plan,
            graph,
            report,
            catalog,
        }
    }

    #[test]
    fn artifact_content_is_deduplicated() {
        let (_directory, store, _project_id) = setup();
        let first = store
            .put_bytes(b"same", Some("one.txt".to_owned()), None)
            .unwrap();
        let second = store
            .put_bytes(b"same", Some("two.txt".to_owned()), None)
            .unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(
            store.artifact_path(&first.sha256).unwrap(),
            store.artifact_path(&second.sha256).unwrap()
        );
        assert_eq!(
            fs::read(store.artifact_path(&first.sha256).unwrap()).unwrap(),
            b"same"
        );
    }

    #[test]
    fn managed_source_provenance_is_durable_and_integrity_checked() {
        let (directory, store, project_id) = setup();
        fs::create_dir_all(directory.path().join("source")).unwrap();
        fs::create_dir_all(directory.path().join("imports")).unwrap();
        let managed_path = directory.path().join("imports").join("demo");
        fs::rename(directory.path().join("source"), &managed_path).unwrap();
        let connection = store.connection().unwrap();
        connection
            .execute(
                "UPDATE projects SET source_path = ?1 WHERE id = ?2",
                params![managed_path.to_string_lossy(), project_id.to_string()],
            )
            .unwrap();
        drop(connection);
        let mut acquisition = SourceAcquisition {
            schema_version: "qbm.source-acquisition/v1".to_owned(),
            id: digest("placeholder"),
            project_id: project_id.clone(),
            kind: SourceAcquisitionKind::UploadedFolder,
            source_locator: "demo-folder".to_owned(),
            repository_id: None,
            requested_revision: None,
            resolved_revision: None,
            retrieval_url: None,
            provider_api_version: None,
            content_sha256: digest("source archive"),
            managed_path,
            total_bytes: 14,
            created_at: Utc::now(),
        };
        acquisition.id = PlatformStore::calculate_source_acquisition_id(&acquisition).unwrap();

        assert_eq!(
            store.save_source_acquisition(&acquisition).unwrap(),
            acquisition
        );
        assert_eq!(
            store.get_source_acquisition(&acquisition.id).unwrap(),
            acquisition
        );
        assert_eq!(
            store.list_source_acquisitions(Some(&project_id)).unwrap(),
            vec![acquisition.clone()]
        );
        let run = store.create_run(project_id, RunMode::Governed).unwrap();
        store.bind_run_acquisition(run.id, &acquisition.id).unwrap();
        assert_eq!(store.get_run_acquisition(run.id).unwrap(), acquisition);
    }

    #[test]
    fn bounded_artifact_reads_enforce_metadata_and_filesystem_size_before_buffering() {
        let (_directory, store, _project_id) = setup();
        let artifact = store
            .put_bytes(b"bounded", Some("bounded.txt".to_owned()), None)
            .unwrap();
        assert!(matches!(
            store.read_artifact_bounded(&artifact.sha256, 6),
            Err(StoreError::ArtifactReadLimit {
                observed_bytes: 7,
                maximum_bytes: 6,
                ..
            })
        ));
        assert_eq!(
            store.read_artifact_bounded(&artifact.sha256, 7).unwrap(),
            b"bounded"
        );

        let connection = store.connection().unwrap();
        connection
            .execute(
                "UPDATE artifacts SET size_bytes = 1 WHERE sha256 = ?1",
                [artifact.sha256.to_string()],
            )
            .unwrap();
        drop(connection);
        assert!(matches!(
            store.read_artifact_bounded(&artifact.sha256, 6),
            Err(StoreError::ArtifactReadLimit {
                observed_bytes: 7,
                maximum_bytes: 6,
                ..
            })
        ));
    }

    #[test]
    fn stale_approval_is_rejected() {
        let (_directory, store, project_id) = setup();
        let run = store.create_run(project_id, RunMode::Governed).unwrap();
        let stage = StageId::new("inventory").unwrap();
        let old_hash = Sha256Digest::new("1".repeat(64)).unwrap();
        let new_hash = Sha256Digest::new("2".repeat(64)).unwrap();
        store
            .record_stage_output(run.id, stage.clone(), old_hash.clone())
            .unwrap();
        store
            .record_stage_output(run.id, stage.clone(), new_hash.clone())
            .unwrap();
        let error = store
            .decide_stage(ApprovalRequest {
                run_id: run.id,
                stage_id: stage,
                expected_output_hash: old_hash,
                decision: Decision::Approve,
                actor_id: ActorId::new("reviewer").unwrap(),
                reason: None,
            })
            .unwrap_err();
        assert!(matches!(
            error,
            StoreError::StaleApproval { current, .. } if current == new_hash
        ));
    }

    #[test]
    fn approved_stage_and_event_chain_are_durable() {
        let (_directory, store, project_id) = setup();
        let run = store.create_run(project_id, RunMode::Onboarding).unwrap();
        let stage = StageId::new("inventory").unwrap();
        let output_hash = Sha256Digest::new("a".repeat(64)).unwrap();
        store
            .record_stage_output(run.id, stage.clone(), output_hash.clone())
            .unwrap();
        store
            .decide_stage(ApprovalRequest {
                run_id: run.id,
                stage_id: stage.clone(),
                expected_output_hash: output_hash,
                decision: Decision::Approve,
                actor_id: ActorId::new("owner").unwrap(),
                reason: Some("reviewed".to_owned()),
            })
            .unwrap();
        assert_eq!(
            store.get_stage_output(run.id, &stage).unwrap().state,
            StageState::Approved
        );
        assert_eq!(store.get_run(run.id).unwrap().state, RunState::Running);
        assert_eq!(store.list_events(run.id).unwrap().len(), 3);
        store.verify_event_chain(run.id).unwrap();
    }

    #[test]
    fn exact_terminal_approval_completes_a_run_idempotently() {
        let (_directory, store, project_id) = setup();
        let run = store.create_run(project_id, RunMode::Governed).unwrap();
        let stage = StageId::new("generic-audit").unwrap();
        let output_hash = digest("generic report");
        store
            .record_stage_output(run.id, stage.clone(), output_hash.clone())
            .unwrap();
        store
            .decide_stage(ApprovalRequest {
                run_id: run.id,
                stage_id: stage.clone(),
                expected_output_hash: output_hash.clone(),
                decision: Decision::Approve,
                actor_id: ActorId::new("owner").unwrap(),
                reason: None,
            })
            .unwrap();

        let completed = store.complete_run(run.id, &stage, &output_hash).unwrap();
        assert_eq!(completed.state, RunState::Complete);
        let event_count = store.list_events(run.id).unwrap().len();
        assert_eq!(
            store.complete_run(run.id, &stage, &output_hash).unwrap(),
            completed
        );
        assert_eq!(store.list_events(run.id).unwrap().len(), event_count);
        store.verify_event_chain(run.id).unwrap();
    }

    #[test]
    fn recording_identical_output_is_idempotent() {
        let (_directory, store, project_id) = setup();
        let run = store.create_run(project_id, RunMode::Governed).unwrap();
        let stage = StageId::new("inventory").unwrap();
        let output_hash = Sha256Digest::new("e".repeat(64)).unwrap();
        store
            .record_stage_output(run.id, stage.clone(), output_hash.clone())
            .unwrap();
        store
            .decide_stage(ApprovalRequest {
                run_id: run.id,
                stage_id: stage.clone(),
                expected_output_hash: output_hash.clone(),
                decision: Decision::Approve,
                actor_id: ActorId::new("owner").unwrap(),
                reason: None,
            })
            .unwrap();
        let before = store.list_events(run.id).unwrap();
        let repeated = store
            .record_stage_output(run.id, stage, output_hash)
            .unwrap();
        assert_eq!(repeated.state, StageState::Approved);
        assert_eq!(store.list_events(run.id).unwrap(), before);
    }

    #[test]
    fn decided_output_rejects_a_second_decision() {
        let (_directory, store, project_id) = setup();
        let run = store.create_run(project_id, RunMode::Governed).unwrap();
        let stage = StageId::new("inventory").unwrap();
        let output_hash = Sha256Digest::new("f".repeat(64)).unwrap();
        store
            .record_stage_output(run.id, stage.clone(), output_hash.clone())
            .unwrap();
        let request = || ApprovalRequest {
            run_id: run.id,
            stage_id: stage.clone(),
            expected_output_hash: output_hash.clone(),
            decision: Decision::Approve,
            actor_id: ActorId::new("owner").unwrap(),
            reason: None,
        };
        store.decide_stage(request()).unwrap();
        let error = store.decide_stage(request()).unwrap_err();
        assert!(matches!(
            error,
            StoreError::StageNotAwaitingApproval {
                state: StageState::Approved,
                ..
            }
        ));
    }

    #[test]
    fn rejection_requires_a_reason() {
        let (_directory, store, project_id) = setup();
        let run = store.create_run(project_id, RunMode::Governed).unwrap();
        let stage = StageId::new("inventory").unwrap();
        let output_hash = Sha256Digest::new("b".repeat(64)).unwrap();
        store
            .record_stage_output(run.id, stage.clone(), output_hash.clone())
            .unwrap();
        let result = store.decide_stage(ApprovalRequest {
            run_id: run.id,
            stage_id: stage,
            expected_output_hash: output_hash,
            decision: Decision::Reject,
            actor_id: ActorId::new("owner").unwrap(),
            reason: None,
        });
        assert!(matches!(
            result,
            Err(StoreError::ReasonRequired(Decision::Reject))
        ));
    }

    #[test]
    fn version_three_database_migrates_to_current_version_and_reopens() {
        let directory = TempDir::new().unwrap();
        let database_path = directory.path().join(DATABASE_FILE);
        let connection = Connection::open(&database_path).unwrap();
        PlatformStore::migrate_v1(&connection).unwrap();
        PlatformStore::migrate_v2(&connection).unwrap();
        PlatformStore::migrate_v3(&connection).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
        drop(connection);

        let store = PlatformStore::open(directory.path()).unwrap();
        let connection = store.connection().unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            DATABASE_SCHEMA_VERSION
        );
        for table in [
            "adapter_conformance_reports",
            "adapters",
            "adapter_detections",
            "run_adapter_detections",
            "adapter_plans",
            "run_adapter_plans",
            "semantic_graphs",
            "run_semantic_graphs",
            "semantic_audit_reports",
            "run_semantic_reports",
            "projection_catalogs",
            "run_projection_catalogs",
            "source_acquisitions",
            "run_acquisitions",
        ] {
            let exists: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "missing migrated table {table}");
        }
        drop(connection);
        drop(store);

        let reopened = PlatformStore::open(directory.path()).unwrap();
        assert_eq!(
            reopened
                .connection()
                .unwrap()
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            DATABASE_SCHEMA_VERSION
        );
    }

    #[test]
    fn adapter_install_requires_source_and_digest_specific_passing_conformance() {
        let (_directory, store, _project_id) = setup();
        let package = adapter_package("oncova", "1.0.0", "Oncology adapter");
        let package_hash = hash_value(&package).unwrap();
        let missing = digest("missing-source");
        let absent_record = AdapterRecord {
            package_hash: package_hash.clone(),
            source_hash: missing,
            conformance_report_id: digest("missing-conformance"),
            package: package.clone(),
            installed_at: Utc::now(),
        };
        assert!(matches!(
            store.save_adapter(&absent_record),
            Err(StoreError::NotFound {
                kind: "artifact",
                ..
            })
        ));

        let source = store
            .put_bytes(b"adapter source", Some("oncova.json".to_owned()), None)
            .unwrap();
        let failing = conformance_report(package_hash.clone(), false, "failing");
        store.save_adapter_conformance(&failing).unwrap();
        let failing_record = AdapterRecord {
            package_hash: package_hash.clone(),
            source_hash: source.sha256.clone(),
            conformance_report_id: failing.id,
            package: package.clone(),
            installed_at: Utc::now(),
        };
        assert!(matches!(
            store.save_adapter(&failing_record),
            Err(StoreError::InvalidRelationship(_))
        ));

        let other_hash = hash_value(&adapter_package("other", "1.0.0", "Other")).unwrap();
        let wrong_digest = conformance_report(other_hash, true, "wrong-package");
        store.save_adapter_conformance(&wrong_digest).unwrap();
        let wrong_digest_record = AdapterRecord {
            conformance_report_id: wrong_digest.id,
            ..failing_record.clone()
        };
        assert!(matches!(
            store.save_adapter(&wrong_digest_record),
            Err(StoreError::InvalidRelationship(_))
        ));

        let passing = conformance_report(package_hash.clone(), true, "passing");
        store.save_adapter_conformance(&passing).unwrap();
        let record = AdapterRecord {
            package_hash: package_hash.clone(),
            source_hash: source.sha256,
            conformance_report_id: passing.id,
            package,
            installed_at: Utc::now(),
        };
        assert_eq!(store.save_adapter(&record).unwrap(), record);
        assert_eq!(store.get_adapter(&package_hash).unwrap(), record);
        assert_eq!(store.list_adapters().unwrap(), vec![record]);
    }

    #[test]
    fn adapter_package_hash_and_version_conflicts_are_rejected() {
        let (_directory, store, _project_id) = setup();
        let first = install_adapter(
            &store,
            adapter_package("pathex", "1.0.0", "First package"),
            "pathex-first",
        );
        assert_eq!(store.save_adapter(&first).unwrap(), first);

        let mut false_hash = first.clone();
        false_hash.package_hash = digest("false-package-hash");
        assert!(matches!(
            store.save_adapter(&false_hash),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("calculated")
        ));

        let conflicting = adapter_package("pathex", "1.0.0", "Changed immutable package");
        let conflicting_hash = hash_value(&conflicting).unwrap();
        let source = store
            .put_bytes(b"changed source", Some("changed.json".to_owned()), None)
            .unwrap();
        let conformance = conformance_report(conflicting_hash.clone(), true, "changed");
        store.save_adapter_conformance(&conformance).unwrap();
        let conflict_record = AdapterRecord {
            package_hash: conflicting_hash.clone(),
            source_hash: source.sha256,
            conformance_report_id: conformance.id,
            package: conflicting,
            installed_at: Utc::now(),
        };
        assert!(matches!(
            store.save_adapter(&conflict_record),
            Err(StoreError::AdapterVersionConflict {
                existing_hash,
                new_hash,
                ..
            }) if existing_hash == first.package_hash && new_hash == conflicting_hash
        ));
    }

    #[test]
    fn identical_adapter_package_can_rotate_its_conformance_certificate() {
        let (_directory, store, _project_id) = setup();
        let installed = install_adapter(
            &store,
            adapter_package("pathex", "1.0.0", "Stable package"),
            "pathex-stable",
        );
        let replacement = conformance_report(
            installed.package_hash.clone(),
            true,
            "new-conformance-suite",
        );
        store.save_adapter_conformance(&replacement).unwrap();

        let unrelated_source = store
            .put_bytes(
                b"a second serialization is not allowed to replace provenance",
                Some("replacement.json".to_owned()),
                None,
            )
            .unwrap();
        let requested = AdapterRecord {
            source_hash: unrelated_source.sha256,
            conformance_report_id: replacement.id.clone(),
            installed_at: Utc::now(),
            ..installed.clone()
        };
        let rotated = store.save_adapter(&requested).unwrap();

        assert_eq!(rotated.package_hash, installed.package_hash);
        assert_eq!(rotated.package, installed.package);
        assert_eq!(rotated.source_hash, installed.source_hash);
        assert_eq!(rotated.installed_at, installed.installed_at);
        assert_eq!(rotated.conformance_report_id, replacement.id);
        assert_eq!(store.get_adapter(&rotated.package_hash).unwrap(), rotated);
    }

    #[test]
    fn adapter_registry_count_is_bounded_before_manifests_are_loaded() {
        let (_directory, store, _project_id) = setup();
        let installed = install_adapter(
            &store,
            adapter_package("bounded", "1.0.0", "Bounded registry"),
            "bounded",
        );

        assert!(matches!(
            store.list_adapters_bounded(0),
            Err(StoreError::RecordReadLimit {
                kind: "adapter registry",
                observed: 1,
                maximum: 0,
            })
        ));
        assert_eq!(store.list_adapters_bounded(1).unwrap(), vec![installed]);
    }

    #[test]
    fn adapter_lineage_and_run_associations_survive_reopen() {
        let (directory, store, project_id) = setup();
        let run = store
            .create_run(project_id.clone(), RunMode::Governed)
            .unwrap();
        let raw_graph = attach_empty_graph(&store, &project_id, run.id, "durable-lineage");
        let adapter = install_adapter(
            &store,
            adapter_package("workflow", "2.0.0", "Workflow adapter"),
            "workflow",
        );
        let lineage = persist_adapter_lineage(&store, run.id, &raw_graph, &adapter, "durable");

        assert_eq!(
            store.list_adapter_detections(run.id).unwrap(),
            vec![lineage.detection.clone()]
        );
        assert_eq!(
            store.list_adapter_plans(run.id).unwrap(),
            vec![lineage.plan.clone()]
        );
        assert_eq!(
            store.list_semantic_graphs(run.id).unwrap(),
            vec![lineage.graph.clone()]
        );
        assert_eq!(
            store.list_semantic_reports(run.id).unwrap(),
            vec![lineage.report.clone()]
        );
        assert_eq!(
            store.list_projection_catalogs(run.id).unwrap(),
            vec![lineage.catalog.clone()]
        );
        drop(store);

        let reopened = PlatformStore::open(directory.path()).unwrap();
        assert_eq!(
            reopened.get_adapter(&adapter.package_hash).unwrap(),
            adapter
        );
        assert_eq!(
            reopened
                .get_adapter_detection(&lineage.detection.id)
                .unwrap(),
            lineage.detection
        );
        assert_eq!(
            reopened.get_adapter_plan(&lineage.plan.id).unwrap(),
            lineage.plan
        );
        assert_eq!(
            reopened.get_semantic_graph(&lineage.graph.id).unwrap(),
            lineage.graph
        );
        assert_eq!(
            reopened.get_semantic_report(&lineage.report.id).unwrap(),
            lineage.report
        );
        assert_eq!(
            reopened
                .get_projection_catalog(&lineage.catalog.id)
                .unwrap(),
            lineage.catalog
        );
    }

    #[test]
    fn adapter_detection_and_plan_reject_binding_mismatches() {
        let (_directory, store, project_id) = setup();
        let first_run = store
            .create_run(project_id.clone(), RunMode::Governed)
            .unwrap();
        let second_run = store
            .create_run(project_id.clone(), RunMode::Governed)
            .unwrap();
        let first_raw = attach_empty_graph(&store, &project_id, first_run.id, "first-raw");
        let second_raw = attach_empty_graph(&store, &project_id, first_run.id, "second-raw");
        let adapter = install_adapter(
            &store,
            adapter_package("finance", "3.0.0", "Finance adapter"),
            "finance",
        );
        let detection = AdapterDetectionReport {
            schema_version: "qbm.adapter-detection/v1".to_owned(),
            id: digest("binding-detection"),
            graph_id: first_raw.id.clone(),
            candidates: vec![candidate(&adapter, true)],
            created_at: Utc::now(),
        };
        assert!(matches!(
            store.save_adapter_detection(second_run.id, &detection),
            Err(StoreError::NotFound {
                kind: "run graph binding",
                ..
            })
        ));
        store
            .save_adapter_detection(first_run.id, &detection)
            .unwrap();

        let lock = adapter_lock(&adapter);
        let valid_plan = AdapterPlan {
            schema_version: "qbm.adapter-plan/v1".to_owned(),
            id: digest("binding-plan"),
            detection_report_id: detection.id.clone(),
            raw_graph_id: first_raw.id.clone(),
            adapters: vec![lock.clone()],
            configuration_hash: digest("binding-config"),
            created_at: Utc::now(),
        };
        assert!(matches!(
            store.save_adapter_plan(second_run.id, &valid_plan),
            Err(StoreError::NotFound {
                kind: "run adapter detection binding",
                ..
            })
        ));
        let wrong_raw_plan = AdapterPlan {
            raw_graph_id: second_raw.id.clone(),
            ..valid_plan.clone()
        };
        assert!(matches!(
            store.save_adapter_plan(first_run.id, &wrong_raw_plan),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("detection graph")
        ));
        let mut wrong_lock = lock.clone();
        wrong_lock.capabilities = BTreeSet::new();
        let wrong_lock_plan = AdapterPlan {
            adapters: vec![wrong_lock],
            ..valid_plan.clone()
        };
        assert!(matches!(
            store.save_adapter_plan(first_run.id, &wrong_lock_plan),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("adapter lock")
        ));
        store.save_adapter_plan(first_run.id, &valid_plan).unwrap();
    }

    #[test]
    fn semantic_outputs_reject_cross_run_and_predecessor_mismatches() {
        let (_directory, store, project_id) = setup();
        let first_run = store
            .create_run(project_id.clone(), RunMode::Governed)
            .unwrap();
        let second_run = store
            .create_run(project_id.clone(), RunMode::Governed)
            .unwrap();
        let first_raw = attach_empty_graph(&store, &project_id, first_run.id, "semantic-first");
        let second_raw = attach_empty_graph(&store, &project_id, first_run.id, "semantic-second");
        let adapter = install_adapter(
            &store,
            adapter_package("education", "1.0.0", "Education adapter"),
            "education",
        );
        let lineage = persist_adapter_lineage(
            &store,
            first_run.id,
            &first_raw,
            &adapter,
            "semantic-binding",
        );

        let wrong_raw_semantic = SemanticGraph {
            raw_graph_id: second_raw.id,
            ..lineage.graph.clone()
        };
        assert!(matches!(
            store.save_semantic_graph(first_run.id, &wrong_raw_semantic),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("adapter plan")
        ));
        let wrong_adapters_semantic = SemanticGraph {
            adapters: Vec::new(),
            ..lineage.graph.clone()
        };
        assert!(matches!(
            store.save_semantic_graph(first_run.id, &wrong_adapters_semantic),
            Err(StoreError::InvalidRelationship(_))
        ));
        let wrong_plan_report = SemanticAuditReport {
            adapter_plan_id: digest("unrelated-plan"),
            ..lineage.report.clone()
        };
        assert!(matches!(
            store.save_semantic_report(first_run.id, &wrong_plan_report),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("graph plan")
        ));
        assert!(matches!(
            store.save_semantic_report(second_run.id, &lineage.report),
            Err(StoreError::NotFound {
                kind: "run semantic graph binding",
                ..
            })
        ));
        let wrong_plan_catalog = ProjectionCatalog {
            adapter_plan_id: digest("catalog-wrong-plan"),
            ..lineage.catalog.clone()
        };
        assert!(matches!(
            store.save_projection_catalog(first_run.id, &wrong_plan_catalog),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("predecessors")
        ));
        assert!(matches!(
            store.save_projection_catalog(second_run.id, &lineage.catalog),
            Err(StoreError::NotFound {
                kind: "run semantic graph binding",
                ..
            })
        ));
    }

    #[test]
    fn adapter_plan_may_only_select_an_eligible_detected_package() {
        let (_directory, store, project_id) = setup();
        let run = store
            .create_run(project_id.clone(), RunMode::Governed)
            .unwrap();
        let raw_graph = attach_empty_graph(&store, &project_id, run.id, "eligibility");
        let eligible = install_adapter(
            &store,
            adapter_package("eligible", "1.0.0", "Detected adapter"),
            "eligible",
        );
        let not_detected = install_adapter(
            &store,
            adapter_package("hidden", "1.0.0", "Not detected adapter"),
            "hidden",
        );
        let detection = AdapterDetectionReport {
            schema_version: "qbm.adapter-detection/v1".to_owned(),
            id: digest("eligibility-detection"),
            graph_id: raw_graph.id.clone(),
            candidates: vec![candidate(&eligible, false)],
            created_at: Utc::now(),
        };
        store.save_adapter_detection(run.id, &detection).unwrap();
        let empty_plan = AdapterPlan {
            schema_version: "qbm.adapter-plan/v1".to_owned(),
            id: digest("empty-plan"),
            detection_report_id: detection.id.clone(),
            raw_graph_id: raw_graph.id.clone(),
            adapters: Vec::new(),
            configuration_hash: digest("empty-config"),
            created_at: Utc::now(),
        };
        assert!(matches!(
            store.save_adapter_plan(run.id, &empty_plan),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("at least one")
        ));
        let plan_for = |label: &str, record: &AdapterRecord| AdapterPlan {
            schema_version: "qbm.adapter-plan/v1".to_owned(),
            id: digest(label),
            detection_report_id: detection.id.clone(),
            raw_graph_id: raw_graph.id.clone(),
            adapters: vec![adapter_lock(record)],
            configuration_hash: digest("eligibility-config"),
            created_at: Utc::now(),
        };
        assert!(matches!(
            store.save_adapter_plan(run.id, &plan_for("ineligible-plan", &eligible)),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("eligible")
        ));
        assert!(matches!(
            store.save_adapter_plan(run.id, &plan_for("undetected-plan", &not_detected)),
            Err(StoreError::InvalidRelationship(message))
                if message.contains("detection")
        ));
    }

    #[test]
    fn modified_adapter_manifest_artifact_is_rejected_on_read() {
        let (_directory, store, _project_id) = setup();
        let adapter = install_adapter(
            &store,
            adapter_package("tamper", "1.0.0", "Tamper evidence"),
            "tamper",
        );
        let connection = store.connection().unwrap();
        let manifest_hash: String = connection
            .query_row(
                "SELECT manifest_hash FROM adapters WHERE package_hash = ?1",
                [adapter.package_hash.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);
        let manifest_hash = Sha256Digest::new(manifest_hash).unwrap();
        fs::write(store.artifact_path(&manifest_hash).unwrap(), b"tampered").unwrap();
        assert!(matches!(
            store.get_adapter(&adapter.package_hash),
            Err(StoreError::ArtifactIntegrity { expected, .. }) if expected == manifest_hash
        ));
    }
}
