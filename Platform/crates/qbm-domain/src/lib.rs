//! Stable domain contracts for Q-BenchMed Platform.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::PathBuf,
    str::FromStr,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

/// Current persisted domain schema version.
pub const DOMAIN_SCHEMA_VERSION: &str = "qbm.domain/v1";
/// Declarative adapter package schema accepted by this release.
pub const ADAPTER_PACKAGE_SCHEMA_VERSION: &str = "qbm.adapter-package/v1";
/// Declarative adapter interpreter API accepted by this release.
pub const ADAPTER_API_VERSION: &str = "qbm.adapter-api/v1";

/// Errors raised while constructing validated domain values.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    /// The supplied identifier does not follow the portable identifier policy.
    #[error("invalid {kind} identifier {value:?}; use 1-64 ASCII letters, digits, '.', '_' or '-'")]
    InvalidIdentifier {
        /// Identifier kind shown to the user.
        kind: &'static str,
        /// Rejected value.
        value: String,
    },
    /// A SHA-256 digest was not exactly 64 hexadecimal characters.
    #[error("invalid SHA-256 digest {0:?}; expected 64 hexadecimal characters")]
    InvalidSha256(String),
    /// A textual enum value was not recognized.
    #[error("unknown {kind} value {value:?}")]
    UnknownValue {
        /// Enum kind shown to the user.
        kind: &'static str,
        /// Rejected value.
        value: String,
    },
}

fn valid_portable_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    value.len() <= 64
        && first.is_ascii_alphanumeric()
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

macro_rules! portable_id {
    ($name:ident, $kind:literal) => {
        #[doc = concat!("Validated ", $kind, " identifier.")]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Construct a validated ", $kind, " identifier.")]
            ///
            /// # Errors
            ///
            /// Returns [`DomainError::InvalidIdentifier`] when the value is
            /// empty, too long, starts with punctuation, or contains a
            /// non-portable character.
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                if valid_portable_id(&value) {
                    Ok(Self(value))
                } else {
                    Err(DomainError::InvalidIdentifier { kind: $kind, value })
                }
            }

            /// Borrow the textual identifier.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

portable_id!(ProjectId, "project");
portable_id!(StageId, "stage");
portable_id!(ActorId, "actor");
portable_id!(AdapterId, "adapter");

/// Opaque globally unique run identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(Uuid);

impl RunId {
    /// Generate a new random run identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for RunId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Opaque globally unique event identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(Uuid);

impl EventId {
    /// Generate a new random event identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for EventId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Opaque globally unique approval identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApprovalId(Uuid);

impl ApprovalId {
    /// Generate a new random approval identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ApprovalId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ApprovalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for ApprovalId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Opaque globally unique intake-inventory identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InventoryId(Uuid);

impl InventoryId {
    /// Generate a new random inventory identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for InventoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for InventoryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for InventoryId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// Validated lowercase SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Validate and normalize a hexadecimal digest.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::InvalidSha256`] unless the value contains
    /// exactly 64 hexadecimal characters.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(DomainError::InvalidSha256(value))
        }
    }

    /// Borrow the normalized digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for Sha256Digest {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// User-selectable orchestration profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// First semantic onboarding; all material stages require review.
    Onboarding,
    /// Minimal static discovery with no execution or network.
    Quick,
    /// Normal adapter-aware audit with capability gates.
    Standard,
    /// Expanded behavioral and performance audit.
    Deep,
    /// Manual governance at every material stage.
    Governed,
    /// Audit a later version against an approved baseline.
    Continuous,
}

impl fmt::Display for RunMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Onboarding => "onboarding",
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Deep => "deep",
            Self::Governed => "governed",
            Self::Continuous => "continuous",
        };
        value.fmt(formatter)
    }
}

impl FromStr for RunMode {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "onboarding" => Ok(Self::Onboarding),
            "quick" => Ok(Self::Quick),
            "standard" => Ok(Self::Standard),
            "deep" => Ok(Self::Deep),
            "governed" => Ok(Self::Governed),
            "continuous" => Ok(Self::Continuous),
            _ => Err(DomainError::UnknownValue {
                kind: "run mode",
                value: value.to_owned(),
            }),
        }
    }
}

/// Durable run state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Work may proceed.
    Running,
    /// A stage decision is required.
    WaitingApproval,
    /// A reviewer requested revised stage output.
    NeedsChanges,
    /// A reviewer rejected the run branch.
    Rejected,
    /// All configured stages completed.
    Complete,
    /// Work was cancelled.
    Cancelled,
    /// The engine could not continue.
    Failed,
}

impl fmt::Display for RunState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::NeedsChanges => "needs_changes",
            Self::Rejected => "rejected",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        };
        value.fmt(formatter)
    }
}

impl FromStr for RunState {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "running" => Ok(Self::Running),
            "waiting_approval" => Ok(Self::WaitingApproval),
            "needs_changes" => Ok(Self::NeedsChanges),
            "rejected" => Ok(Self::Rejected),
            "complete" => Ok(Self::Complete),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            _ => Err(DomainError::UnknownValue {
                kind: "run state",
                value: value.to_owned(),
            }),
        }
    }
}

/// State of a material stage output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageState {
    /// Output exists and awaits a decision.
    WaitingApproval,
    /// Exact output was approved.
    Approved,
    /// Exact output was rejected.
    Rejected,
    /// Reviewer requested revised output.
    NeedsChanges,
}

impl fmt::Display for StageState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::WaitingApproval => "waiting_approval",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::NeedsChanges => "needs_changes",
        };
        value.fmt(formatter)
    }
}

impl FromStr for StageState {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "waiting_approval" => Ok(Self::WaitingApproval),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "needs_changes" => Ok(Self::NeedsChanges),
            _ => Err(DomainError::UnknownValue {
                kind: "stage state",
                value: value.to_owned(),
            }),
        }
    }
}

/// Manual decision on an exact stage output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Permit dependent work to continue.
    Approve,
    /// Terminate the current run branch.
    Reject,
    /// Require a revised stage output.
    RequestChanges,
}

impl fmt::Display for Decision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
            Self::RequestChanges => "request_changes",
        };
        value.fmt(formatter)
    }
}

impl FromStr for Decision {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "approve" => Ok(Self::Approve),
            "reject" => Ok(Self::Reject),
            "request_changes" | "request-changes" => Ok(Self::RequestChanges),
            _ => Err(DomainError::UnknownValue {
                kind: "decision",
                value: value.to_owned(),
            }),
        }
    }
}

/// Registered project metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Schema used to serialize this record.
    pub schema_version: String,
    /// Stable project identifier.
    pub id: ProjectId,
    /// User-facing name.
    pub display_name: String,
    /// Canonical source directory or archive registered read-only.
    pub source_path: PathBuf,
    /// Creation time.
    pub created_at: DateTime<Utc>,
}

/// How a browser or API client supplied a project's source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAcquisitionKind {
    /// One archive uploaded from the user's device.
    UploadedArchive,
    /// A browser-selected folder uploaded as bounded individual files.
    UploadedFolder,
    /// An immutable commit archive downloaded from a public GitHub repository.
    PublicGithub,
}

/// Durable provenance for source material copied into platform-managed storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceAcquisition {
    /// Schema used to serialize this record.
    pub schema_version: String,
    /// Canonical identity of the provider/source/version/content fields.
    pub id: Sha256Digest,
    /// Project whose source path points at the managed copy.
    pub project_id: ProjectId,
    /// Acquisition mechanism.
    pub kind: SourceAcquisitionKind,
    /// Safe display locator: an upload name or canonical public repository URL.
    pub source_locator: String,
    /// Stable upstream repository identity when the provider exposes one.
    pub repository_id: Option<u64>,
    /// User-requested Git reference, when supplied.
    pub requested_revision: Option<String>,
    /// Immutable resolved revision, such as a full Git commit SHA.
    pub resolved_revision: Option<String>,
    /// Validated final retrieval URL for a remote source archive.
    pub retrieval_url: Option<String>,
    /// Remote API contract version used to resolve the source.
    pub provider_api_version: Option<String>,
    /// SHA-256 of the archive or canonical folder-upload manifest.
    pub content_sha256: Sha256Digest,
    /// Canonical source path below the platform's managed import directory.
    pub managed_path: PathBuf,
    /// Total uploaded or downloaded source bytes.
    pub total_bytes: u64,
    /// Time the managed source was committed.
    pub created_at: DateTime<Utc>,
}

/// Durable audit run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Schema used to serialize this record.
    pub schema_version: String,
    /// Unique run identifier.
    pub id: RunId,
    /// Project under audit.
    pub project_id: ProjectId,
    /// Selected orchestration profile.
    pub mode: RunMode,
    /// Current state.
    pub state: RunState,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last durable state change.
    pub updated_at: DateTime<Utc>,
}

/// Latest output recorded for one stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageOutput {
    /// Run containing the stage.
    pub run_id: RunId,
    /// Stable stage name.
    pub stage_id: StageId,
    /// Content hash approved or rejected by a reviewer.
    pub output_hash: Sha256Digest,
    /// Current decision state.
    pub state: StageState,
    /// Time the output became current.
    pub updated_at: DateTime<Utc>,
}

/// One append-only, tamper-evident run event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEvent {
    /// Unique event identifier.
    pub id: EventId,
    /// Parent run.
    pub run_id: RunId,
    /// Monotonic per-run sequence starting at one.
    pub sequence: u64,
    /// Stable event kind.
    pub kind: String,
    /// Optional material stage.
    pub stage_id: Option<StageId>,
    /// Structured event-specific data.
    pub payload: Value,
    /// Event creation time.
    pub created_at: DateTime<Utc>,
    /// Hash of the preceding event, if any.
    pub previous_hash: Option<Sha256Digest>,
    /// Hash of this canonical event identity.
    pub event_hash: Sha256Digest,
}

/// Manual decision bound to an exact stage output hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approval {
    /// Unique decision identifier.
    pub id: ApprovalId,
    /// Parent run.
    pub run_id: RunId,
    /// Reviewed stage.
    pub stage_id: StageId,
    /// Exact reviewed output.
    pub stage_output_hash: Sha256Digest,
    /// Decision.
    pub decision: Decision,
    /// Reviewer identity.
    pub actor_id: ActorId,
    /// Required explanation for rejection/change; optional for approval.
    pub reason: Option<String>,
    /// Decision time.
    pub created_at: DateTime<Utc>,
}

/// Content-addressed stored artifact metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    /// Content identity.
    pub sha256: Sha256Digest,
    /// Number of bytes.
    pub size_bytes: u64,
    /// Relative path inside the platform artifact store.
    pub storage_path: PathBuf,
    /// Optional original file name for display only.
    pub original_name: Option<String>,
    /// Optional declared media type.
    pub media_type: Option<String>,
    /// First time this content was committed.
    pub created_at: DateTime<Utc>,
}

/// Kind of source presented to the intake engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntakeSourceKind {
    /// Existing local directory.
    Directory,
    /// ZIP archive read without extracting into the source tree.
    ZipArchive,
    /// Uncompressed TAR archive.
    TarArchive,
    /// Gzip-compressed TAR archive.
    TarGzArchive,
}

impl fmt::Display for IntakeSourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Directory => "directory",
            Self::ZipArchive => "zip_archive",
            Self::TarArchive => "tar_archive",
            Self::TarGzArchive => "tar_gz_archive",
        };
        value.fmt(formatter)
    }
}

/// Filesystem or archive entry kind observed during intake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryEntryKind {
    /// Regular file eligible for hashing and freezing.
    File,
    /// Directory container.
    Directory,
    /// Symbolic link, never followed by default.
    Symlink,
    /// Other filesystem/archive object such as a device or hard link.
    Special,
}

/// Intake decision for one observed entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryDisposition {
    /// File content was hashed and frozen in the artifact store.
    Included,
    /// Entry was deliberately omitted by policy.
    Excluded,
    /// Entry violated a safety boundary and prevents a complete intake.
    Blocked,
}

/// Versioned safety and resource policy used for one inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakePolicy {
    /// Schema used to serialize this policy.
    pub schema_version: String,
    /// Directory/file names skipped at any depth.
    pub excluded_names: BTreeSet<String>,
    /// Normalized relative path prefixes skipped with their descendants.
    pub excluded_prefixes: BTreeSet<String>,
    /// Optional normalized prefixes that bound included content.
    pub included_prefixes: BTreeSet<String>,
    /// Maximum observed entries, including excluded entries.
    pub max_entries: u64,
    /// Maximum total included uncompressed bytes.
    pub max_total_bytes: u64,
    /// Maximum included bytes for one file.
    pub max_single_file_bytes: u64,
    /// Maximum directory/archive nesting depth.
    pub max_depth: u32,
    /// Maximum uncompressed/compressed size ratio for archive members.
    pub max_compression_ratio: u64,
}

impl Default for IntakePolicy {
    fn default() -> Self {
        Self {
            schema_version: "qbm.intake-policy/v1".to_owned(),
            excluded_names: [
                ".DS_Store",
                ".git",
                ".hg",
                ".qbenchmed",
                ".svn",
                "__pycache__",
                "node_modules",
                "target",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            excluded_prefixes: BTreeSet::new(),
            included_prefixes: BTreeSet::new(),
            max_entries: 250_000,
            max_total_bytes: 50 * 1024 * 1024 * 1024,
            max_single_file_bytes: 4 * 1024 * 1024 * 1024,
            max_depth: 128,
            max_compression_ratio: 1_000,
        }
    }
}

/// One deterministic inventory entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry {
    /// Normalized `/`-separated path relative to the intake root.
    pub relative_path: String,
    /// Observed object kind.
    pub kind: InventoryEntryKind,
    /// Policy decision.
    pub disposition: InventoryDisposition,
    /// Included uncompressed byte size, or declared size for excluded files.
    pub size_bytes: u64,
    /// Included content identity.
    pub sha256: Option<Sha256Digest>,
    /// Machine-readable omission or block reason.
    pub reason: Option<String>,
}

/// Deterministic inventory and its content identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    /// Schema used to serialize this manifest.
    pub schema_version: String,
    /// Per-scan inventory record ID.
    pub id: InventoryId,
    /// Run that requested this intake.
    pub run_id: RunId,
    /// Registered project.
    pub project_id: ProjectId,
    /// Directory or archive source kind.
    pub source_kind: IntakeSourceKind,
    /// Canonical local source path.
    pub source_path: PathBuf,
    /// Hash of the source archive itself, when applicable.
    pub source_container_hash: Option<Sha256Digest>,
    /// Exact policy identity.
    pub policy_hash: Sha256Digest,
    /// Canonical identity of all deterministic inventory fields and entries.
    pub inventory_hash: Sha256Digest,
    /// Entries sorted by normalized relative path.
    pub entries: Vec<InventoryEntry>,
    /// Number of included regular files.
    pub included_files: u64,
    /// Number of deliberately excluded entries.
    pub excluded_entries: u64,
    /// Number of safety-blocked entries.
    pub blocked_entries: u64,
    /// Total included uncompressed bytes.
    pub total_included_bytes: u64,
    /// Creation time, excluded from content identity.
    pub created_at: DateTime<Utc>,
}

/// One path-to-content binding in an immutable source snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotArtifact {
    /// Normalized relative source path.
    pub relative_path: String,
    /// Content identity in the artifact store.
    pub sha256: Sha256Digest,
    /// Content size.
    pub size_bytes: u64,
}

/// Immutable project source snapshot created from an approved inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSnapshot {
    /// Schema used to serialize this snapshot.
    pub schema_version: String,
    /// Snapshot content identity.
    pub id: Sha256Digest,
    /// Exact inventory content identity.
    pub inventory_hash: Sha256Digest,
    /// Exact intake policy identity.
    pub policy_hash: Sha256Digest,
    /// Registered project.
    pub project_id: ProjectId,
    /// Included path/content bindings sorted by path.
    pub artifacts: Vec<SnapshotArtifact>,
    /// Included byte total.
    pub total_bytes: u64,
    /// Creation time, excluded from snapshot identity.
    pub created_at: DateTime<Utc>,
}

/// Parser selected for one immutable source artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    /// JSON document.
    Json,
    /// YAML document or multi-document stream.
    Yaml,
    /// TOML document.
    Toml,
    /// Rust source parsed as syntax only and never compiled or executed.
    Rust,
    /// Python source parsed statically and never imported or executed.
    Python,
    /// Content retained as a file node but not structurally interpreted.
    Unknown,
}

/// Language-neutral raw graph node category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    /// Synthetic project root.
    Project,
    /// One immutable source file.
    File,
    /// Structured document object/map.
    Object,
    /// Structured document sequence.
    Array,
    /// Structured scalar carrying an identifier or explicit reference.
    Scalar,
    /// Rust module declaration.
    RustModule,
    /// Named Rust syntax item.
    RustItem,
    /// Python module derived from one immutable source path.
    PythonModule,
    /// Python class declaration.
    PythonClass,
    /// Python function or method declaration.
    PythonFunction,
    /// Python test class, function, or method declaration.
    PythonTest,
    /// Python call expression retained as a static, unresolved fact when needed.
    PythonCall,
}

/// Language-neutral graph edge category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeKind {
    /// Structural parent-child relationship.
    Contains,
    /// Explicit declarative reference.
    References,
    /// Rust `use` or Python import relationship.
    Imports,
    /// Rust external module declaration.
    DeclaresModule,
    /// Statically observed Python call relationship.
    Calls,
}

/// Precise source evidence attached to graph elements and findings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EvidenceLocation {
    /// Frozen artifact containing the evidence, absent only for synthetic nodes.
    pub artifact_sha256: Option<Sha256Digest>,
    /// Normalized source path, absent only for synthetic nodes.
    pub relative_path: Option<String>,
    /// JSON Pointer or scanner-specific syntax path.
    pub pointer: Option<String>,
    /// Optional one-based source line.
    pub line: Option<u64>,
    /// Optional one-based source column.
    pub column: Option<u64>,
}

/// One deterministic node in the raw project graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Canonical node identity.
    pub id: Sha256Digest,
    /// Node category.
    pub kind: GraphNodeKind,
    /// Short user-facing label.
    pub label: String,
    /// Parser used for the source file.
    pub format: SourceFormat,
    /// Explicit or container-derived identifier when available.
    pub identifier: Option<String>,
    /// Whether generic reachability policy treats this as a definition.
    pub is_definition: bool,
    /// Small scanner-neutral properties used by adapters and policies.
    pub properties: BTreeMap<String, Value>,
    /// Immutable source provenance.
    pub evidence: EvidenceLocation,
}

/// One deterministic edge in the raw project graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Canonical edge identity.
    pub id: Sha256Digest,
    /// Origin node.
    pub from: Sha256Digest,
    /// Resolved destination, absent for unresolved/ambiguous references.
    pub to: Option<Sha256Digest>,
    /// Relationship category.
    pub kind: GraphEdgeKind,
    /// Original textual reference for semantic edges.
    pub reference: Option<String>,
    /// Immutable source provenance.
    pub evidence: EvidenceLocation,
}

/// Severity for scanner diagnostics and audit findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Informational optimization or coverage observation.
    Info,
    /// Likely maintainability or completeness issue.
    Warning,
    /// Defect likely to affect correctness.
    Error,
    /// Defect that invalidates safe audit interpretation.
    Critical,
}

/// Non-fatal scanner observation retained in the graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanDiagnostic {
    /// Stable machine-readable diagnostic code.
    pub code: String,
    /// Diagnostic severity.
    pub severity: Severity,
    /// Human-readable explanation.
    pub message: String,
    /// Immutable source provenance.
    pub evidence: EvidenceLocation,
}

/// Versioned resource and interpretation policy for the generic graph builder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPolicy {
    /// Schema used to serialize this policy.
    pub schema_version: String,
    /// Maximum bytes parsed from one supported source file.
    pub max_parse_file_bytes: u64,
    /// Maximum nodes in one raw graph.
    pub max_nodes: u64,
    /// Maximum edges in one raw graph.
    pub max_edges: u64,
    /// Object keys treated as identifiers.
    pub identifier_keys: BTreeSet<String>,
    /// Object keys treated as explicit references.
    pub reference_keys: BTreeSet<String>,
    /// Object keys whose children are generic definitions.
    pub definition_container_keys: BTreeSet<String>,
}

impl Default for GraphPolicy {
    fn default() -> Self {
        Self {
            schema_version: "qbm.graph-policy/v1".to_owned(),
            max_parse_file_bytes: 16 * 1024 * 1024,
            max_nodes: 1_000_000,
            max_edges: 2_000_000,
            identifier_keys: ["$id", "id"].into_iter().map(str::to_owned).collect(),
            reference_keys: ["$ref", "depends_on", "extends", "ref", "reference"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            definition_container_keys: [
                "$defs",
                "components",
                "definitions",
                "nodes",
                "rules",
                "workflows",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }
}

/// Deterministic scanner-neutral project graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawProjectGraph {
    /// Schema used to serialize this graph.
    pub schema_version: String,
    /// Canonical graph identity.
    pub id: Sha256Digest,
    /// Approved source snapshot consumed by the builder.
    pub snapshot_id: Sha256Digest,
    /// Registered project.
    pub project_id: ProjectId,
    /// Exact graph-policy identity.
    pub policy_hash: Sha256Digest,
    /// Nodes sorted by identity.
    pub nodes: Vec<GraphNode>,
    /// Edges sorted by identity.
    pub edges: Vec<GraphEdge>,
    /// Non-fatal parser diagnostics sorted deterministically.
    pub diagnostics: Vec<ScanDiagnostic>,
    /// Creation time, excluded from graph identity.
    pub created_at: DateTime<Utc>,
}

/// One actionable, evidence-linked generic audit finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFinding {
    /// Canonical finding identity.
    pub id: Sha256Digest,
    /// Stable machine-readable policy code.
    pub code: String,
    /// Finding severity.
    pub severity: Severity,
    /// Short title.
    pub title: String,
    /// Detailed explanation.
    pub message: String,
    /// Concrete remediation guidance.
    pub remediation: String,
    /// Related graph node identities.
    pub affected_nodes: Vec<Sha256Digest>,
    /// Immutable evidence locations.
    pub evidence: Vec<EvidenceLocation>,
}

/// Finding totals by severity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSummary {
    /// Total findings.
    pub total: u64,
    /// Informational findings.
    pub info: u64,
    /// Warning findings.
    pub warnings: u64,
    /// Error findings.
    pub errors: u64,
    /// Critical findings.
    pub critical: u64,
}

/// Deterministic generic structural audit report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReport {
    /// Schema used to serialize this report.
    pub schema_version: String,
    /// Canonical report identity.
    pub id: Sha256Digest,
    /// Approved raw graph audited by the policies.
    pub graph_id: Sha256Digest,
    /// Registered project.
    pub project_id: ProjectId,
    /// Findings sorted by severity/code/identity.
    pub findings: Vec<AuditFinding>,
    /// Finding totals.
    pub summary: AuditSummary,
    /// Creation time, excluded from report identity.
    pub created_at: DateTime<Utc>,
}

/// Declared capability of a purely declarative adapter package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterCapability {
    /// Read the exact approved raw graph.
    RawGraphRead,
    /// Resolve scalar text values only from artifacts referenced by that graph.
    SnapshotArtifactTextRead,
    /// Emit bounded semantic nodes.
    EmitSemanticNodes,
    /// Emit bounded semantic relationships.
    EmitSemanticEdges,
    /// Emit bounded adapter-attributed findings.
    EmitFindings,
    /// Publish named filtered semantic views.
    EmitProjections,
}

/// Immutable package metadata shown in registries and reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterManifest {
    /// Manifest schema.
    pub schema_version: String,
    /// Declarative interpreter API version.
    pub adapter_api_version: String,
    /// Stable adapter identifier.
    pub id: AdapterId,
    /// Adapter-controlled version validated by the conformance kit.
    pub version: String,
    /// Human-readable adapter name.
    pub display_name: String,
    /// Short purpose and supported-domain description.
    pub description: String,
    /// Minimum platform domain schema expected by this package.
    pub minimum_platform_schema: String,
    /// Declared declarative capabilities.
    pub capabilities: BTreeSet<AdapterCapability>,
    /// SPDX licence expression or other concise licence identifier.
    pub license: String,
}

/// Scanner-neutral selector used by detection and mapping rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawNodeSelector {
    /// Allowed source formats; empty means any.
    pub formats: BTreeSet<SourceFormat>,
    /// Allowed node kinds; empty means any.
    pub kinds: BTreeSet<GraphNodeKind>,
    /// Required normalized source-path prefixes; empty means any.
    pub path_prefixes: BTreeSet<String>,
    /// Required normalized source-path suffixes; empty means any.
    pub path_suffixes: BTreeSet<String>,
    /// Exact allowed labels; empty means any.
    pub labels: BTreeSet<String>,
    /// Exact allowed identifiers; empty means any.
    pub identifiers: BTreeSet<String>,
    /// Require presence (`true`) or absence (`false`) of any identifier.
    pub identifier_required: Option<bool>,
    /// Restrict generic definition status.
    pub definition: Option<bool>,
}

/// One weighted adapter-detection rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterDetectionRule {
    /// Package-local rule identifier.
    pub id: String,
    /// Reviewer-facing explanation.
    pub description: String,
    /// Raw graph selector.
    pub selector: RawNodeSelector,
    /// Minimum matching raw nodes for the rule to match.
    pub minimum_matches: u64,
    /// Positive contribution to candidate confidence.
    pub weight: u32,
    /// Whether failure makes the candidate ineligible.
    pub required: bool,
}

/// Source for a semantic node's stable external identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticIdentitySource {
    /// Raw graph node hash.
    RawNodeId,
    /// Raw node identifier, falling back to the raw node hash.
    Identifier,
    /// Raw node label.
    Label,
    /// Normalized source path and syntax pointer.
    SourceLocation,
}

/// Declarative value source for a mapped semantic attribute.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum MappingValueSource {
    /// Raw node identifier.
    Identifier,
    /// Raw node label.
    Label,
    /// Normalized source path.
    RelativePath,
    /// Scanner pointer.
    Pointer,
    /// Scalar value resolved on demand from the frozen artifact and source pointer.
    EvidenceValue,
    /// One small scanner-neutral raw property.
    RawProperty {
        /// Property key.
        key: String,
    },
    /// Constant package-declared JSON value.
    Literal {
        /// Constant value.
        value: Value,
    },
}

/// One target semantic attribute binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingAttribute {
    /// Portable semantic attribute name.
    pub target: String,
    /// Declarative source of the value.
    pub value: MappingValueSource,
    /// Whether a missing source value prevents this mapping.
    pub required: bool,
}

/// Map matching raw nodes into one semantic node type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeMappingRule {
    /// Package-local rule identifier.
    pub id: String,
    /// Raw node selector.
    pub selector: RawNodeSelector,
    /// Portable semantic node type.
    pub semantic_type: String,
    /// External identity source.
    pub identity_source: SemanticIdentitySource,
    /// Attribute bindings.
    pub attributes: Vec<MappingAttribute>,
}

/// Map matching raw edges between mapped nodes into semantic relationships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeMappingRule {
    /// Package-local rule identifier.
    pub id: String,
    /// Allowed raw edge kinds; empty means any semantic edge.
    pub raw_kinds: BTreeSet<GraphEdgeKind>,
    /// Optional required source semantic type.
    pub from_semantic_type: Option<String>,
    /// Optional required destination semantic type.
    pub to_semantic_type: Option<String>,
    /// Portable semantic relationship type.
    pub relation_type: String,
}

/// Declarative raw-to-semantic mapping pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingPack {
    /// Mapping schema.
    pub schema_version: String,
    /// Node mapping rules.
    pub nodes: Vec<NodeMappingRule>,
    /// Edge mapping rules.
    pub edges: Vec<EdgeMappingRule>,
}

/// Declarative semantic policy assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "assertion", rename_all = "snake_case", deny_unknown_fields)]
pub enum SemanticPolicyAssertion {
    /// Require at least a count of one semantic node type.
    MinimumTypeCount {
        /// Semantic node type.
        semantic_type: String,
        /// Inclusive lower bound.
        minimum: u64,
    },
    /// Limit a semantic node type count.
    MaximumTypeCount {
        /// Semantic node type.
        semantic_type: String,
        /// Inclusive upper bound.
        maximum: u64,
    },
    /// Require an attribute on every node of a type.
    RequiredAttribute {
        /// Semantic node type.
        semantic_type: String,
        /// Attribute name.
        attribute: String,
    },
    /// Require external IDs to be unique within a type.
    UniqueExternalId {
        /// Semantic node type.
        semantic_type: String,
    },
    /// Require every node of a type to have a matching outgoing relationship.
    RequiredOutgoingRelation {
        /// Semantic node type.
        semantic_type: String,
        /// Relationship type.
        relation_type: String,
    },
    /// Forbid self-relationships of one type.
    ForbidSelfRelation {
        /// Relationship type.
        relation_type: String,
    },
}

/// One evidence-producing semantic policy rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPolicyRule {
    /// Package-local rule identifier and finding-code suffix.
    pub id: String,
    /// Finding severity.
    pub severity: Severity,
    /// Short finding title.
    pub title: String,
    /// Reviewer-facing purpose.
    pub description: String,
    /// Concrete remediation guidance.
    pub remediation: String,
    /// Declarative assertion.
    pub assertion: SemanticPolicyAssertion,
}

/// Declarative semantic policy pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPolicyPack {
    /// Policy schema.
    pub schema_version: String,
    /// Rules evaluated in deterministic ID order.
    pub rules: Vec<SemanticPolicyRule>,
}

/// Named filtered semantic view published by an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionDefinition {
    /// Package-local projection identifier.
    pub id: String,
    /// User-facing name.
    pub display_name: String,
    /// Purpose of the view.
    pub description: String,
    /// Included semantic node types; empty means all.
    pub semantic_types: BTreeSet<String>,
    /// Included relationship types; empty means all between included nodes.
    pub relation_types: BTreeSet<String>,
}

/// Lightweight raw node used by declarative adapter conformance fixtures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterFixtureNode {
    /// Fixture-local node key.
    pub key: String,
    /// Raw node kind.
    pub kind: GraphNodeKind,
    /// Source format.
    pub format: SourceFormat,
    /// Node label.
    pub label: String,
    /// Optional identifier.
    pub identifier: Option<String>,
    /// Definition marker.
    pub is_definition: bool,
    /// Synthetic relative path.
    pub relative_path: String,
    /// Synthetic scanner pointer.
    pub pointer: Option<String>,
    /// Small scanner-neutral properties.
    pub properties: BTreeMap<String, Value>,
    /// Optional scalar value returned by the fixture evidence resolver.
    pub evidence_value: Option<Value>,
}

/// Lightweight raw edge used by declarative adapter conformance fixtures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterFixtureEdge {
    /// Fixture-local source node key.
    pub from: String,
    /// Optional fixture-local resolved target key.
    pub to: Option<String>,
    /// Raw relationship kind.
    pub kind: GraphEdgeKind,
    /// Optional original reference text.
    pub reference: Option<String>,
}

/// Expected output for one adapter conformance fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterFixtureExpectation {
    /// Expected detection eligibility.
    pub detection_eligible: bool,
    /// Minimum acceptable detection confidence in basis points.
    pub minimum_confidence_bps: u16,
    /// Exact semantic node counts for named types.
    pub semantic_type_counts: BTreeMap<String, u64>,
    /// Exact semantic relationship counts for named types.
    pub semantic_relation_counts: BTreeMap<String, u64>,
    /// Exact finding-code set expected from semantic policy evaluation.
    pub finding_codes: BTreeSet<String>,
    /// Exact expected availability for package-local projection IDs.
    pub projection_availability: BTreeMap<String, bool>,
}

/// One self-contained declarative conformance fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterFixture {
    /// Package-local fixture identifier.
    pub id: String,
    /// Synthetic raw nodes.
    pub nodes: Vec<AdapterFixtureNode>,
    /// Synthetic raw edges.
    pub edges: Vec<AdapterFixtureEdge>,
    /// Expected detection, mapping, and policy behavior.
    pub expect: AdapterFixtureExpectation,
}

/// Complete portable declarative adapter package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterPackage {
    /// Package schema.
    pub schema_version: String,
    /// Registry metadata.
    pub manifest: AdapterManifest,
    /// Weighted detection rules.
    pub detection: Vec<AdapterDetectionRule>,
    /// Raw-to-semantic mapping pack.
    pub mappings: MappingPack,
    /// Semantic policy pack.
    pub policies: SemanticPolicyPack,
    /// Named projection catalog.
    pub projections: Vec<ProjectionDefinition>,
    /// Self-contained conformance fixtures.
    pub fixtures: Vec<AdapterFixture>,
}

/// Installed immutable adapter registry record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterRecord {
    /// Canonical package content hash.
    pub package_hash: Sha256Digest,
    /// Hash of the original imported JSON bytes for supply-chain evidence.
    pub source_hash: Sha256Digest,
    /// Digest-specific conformance result used to enable the package.
    pub conformance_report_id: Sha256Digest,
    /// Validated package.
    pub package: AdapterPackage,
    /// Installation time, excluded from package identity.
    pub installed_at: DateTime<Utc>,
}

/// One detection-rule outcome for a candidate adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionRuleOutcome {
    /// Detection rule ID.
    pub rule_id: String,
    /// Matching raw node count.
    pub match_count: u64,
    /// Whether the minimum was met.
    pub matched: bool,
    /// Whether the rule was required.
    pub required: bool,
    /// Rule weight.
    pub weight: u32,
    /// Deterministic bounded sample of matching raw node identities.
    pub matching_node_samples: Vec<Sha256Digest>,
}

/// Detection score for one installed adapter version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterCandidate {
    /// Adapter ID.
    pub adapter_id: AdapterId,
    /// Adapter version.
    pub adapter_version: String,
    /// Exact installed package hash.
    pub package_hash: Sha256Digest,
    /// Eligibility after required rules.
    pub eligible: bool,
    /// Confidence in basis points from zero to ten thousand.
    pub confidence_bps: u16,
    /// Sum of matched rule weights.
    pub matched_weight: u64,
    /// Sum of all rule weights.
    pub maximum_weight: u64,
    /// Per-rule evidence.
    pub rules: Vec<DetectionRuleOutcome>,
}

/// Deterministic adapter detection report over an approved raw graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterDetectionReport {
    /// Report schema.
    pub schema_version: String,
    /// Canonical detection identity.
    pub id: Sha256Digest,
    /// Approved raw graph.
    pub graph_id: Sha256Digest,
    /// Candidates sorted by eligibility, confidence, ID, and version.
    pub candidates: Vec<AdapterCandidate>,
    /// Creation time, excluded from identity.
    pub created_at: DateTime<Utc>,
}

/// Exact immutable adapter package selected in an approved plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterLock {
    /// Adapter ID.
    pub adapter_id: AdapterId,
    /// Adapter version label.
    pub adapter_version: String,
    /// Authoritative normalized package hash.
    pub package_hash: Sha256Digest,
    /// Conformance certificate locked into the plan.
    pub conformance_report_id: Sha256Digest,
    /// Capabilities granted by the declarative package.
    pub capabilities: BTreeSet<AdapterCapability>,
}

/// Deterministic adapter selection plan locked before semantic projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterPlan {
    /// Plan schema.
    pub schema_version: String,
    /// Canonical plan identity.
    pub id: Sha256Digest,
    /// Approved detection report.
    pub detection_report_id: Sha256Digest,
    /// Approved raw graph.
    pub raw_graph_id: Sha256Digest,
    /// Selected packages sorted by adapter ID, version, and hash.
    pub adapters: Vec<AdapterLock>,
    /// Canonical configuration hash; initially the empty configuration object.
    pub configuration_hash: Sha256Digest,
    /// Creation time, excluded from identity.
    pub created_at: DateTime<Utc>,
}

/// One adapter-projected semantic node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticNode {
    /// Canonical semantic node identity.
    pub id: Sha256Digest,
    /// Adapter that emitted the node.
    pub adapter_id: AdapterId,
    /// Exact package that emitted the node.
    pub package_hash: Sha256Digest,
    /// Mapping rule that emitted the node.
    pub mapping_rule_id: String,
    /// Portable semantic type.
    pub semantic_type: String,
    /// Stable adapter-selected external identity.
    pub external_id: String,
    /// User-facing label.
    pub label: String,
    /// Declaratively mapped attributes.
    pub attributes: BTreeMap<String, Value>,
    /// Origin raw node.
    pub source_node: Sha256Digest,
    /// Immutable source evidence.
    pub evidence: Vec<EvidenceLocation>,
}

/// One adapter-projected semantic relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticEdge {
    /// Canonical semantic edge identity.
    pub id: Sha256Digest,
    /// Adapter that emitted the relationship.
    pub adapter_id: AdapterId,
    /// Exact package that emitted the relationship.
    pub package_hash: Sha256Digest,
    /// Mapping rule that emitted the relationship.
    pub mapping_rule_id: String,
    /// Source semantic node.
    pub from: Sha256Digest,
    /// Destination semantic node.
    pub to: Sha256Digest,
    /// Portable relationship type.
    pub relation_type: String,
    /// Origin raw edge.
    pub source_edge: Sha256Digest,
    /// Immutable source evidence.
    pub evidence: Vec<EvidenceLocation>,
}

/// Deterministic semantic graph produced by one exact immutable adapter plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticGraph {
    /// Graph schema.
    pub schema_version: String,
    /// Canonical graph identity.
    pub id: Sha256Digest,
    /// Approved raw graph.
    pub raw_graph_id: Sha256Digest,
    /// Approved adapter plan.
    pub adapter_plan_id: Sha256Digest,
    /// Exact selected packages.
    pub adapters: Vec<AdapterLock>,
    /// Semantic nodes sorted by identity.
    pub nodes: Vec<SemanticNode>,
    /// Semantic edges sorted by identity.
    pub edges: Vec<SemanticEdge>,
    /// Creation time, excluded from identity.
    pub created_at: DateTime<Utc>,
}

/// Deterministic adapter policy report over a semantic graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticAuditReport {
    /// Report schema.
    pub schema_version: String,
    /// Canonical report identity.
    pub id: Sha256Digest,
    /// Approved semantic graph.
    pub semantic_graph_id: Sha256Digest,
    /// Approved adapter plan.
    pub adapter_plan_id: Sha256Digest,
    /// Evidence-linked policy findings.
    pub findings: Vec<AuditFinding>,
    /// Severity totals.
    pub summary: AuditSummary,
    /// Creation time, excluded from identity.
    pub created_at: DateTime<Utc>,
}

/// One resolved named semantic projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProjection {
    /// Adapter that declared the projection.
    pub adapter_id: AdapterId,
    /// Exact package that declared the projection.
    pub package_hash: Sha256Digest,
    /// Package-local projection ID.
    pub projection_id: String,
    /// User-facing name.
    pub display_name: String,
    /// Purpose of the view.
    pub description: String,
    /// Whether all declared semantic types and relationships are available.
    pub available: bool,
    /// Deterministic reasons for unavailability.
    pub unavailable_reasons: Vec<String>,
    /// Included semantic node IDs.
    pub nodes: Vec<Sha256Digest>,
    /// Included semantic edge IDs.
    pub edges: Vec<Sha256Digest>,
}

/// Deterministic catalog of adapter-defined semantic views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionCatalog {
    /// Catalog schema.
    pub schema_version: String,
    /// Canonical catalog identity.
    pub id: Sha256Digest,
    /// Approved semantic graph.
    pub semantic_graph_id: Sha256Digest,
    /// Approved semantic audit report.
    pub semantic_report_id: Sha256Digest,
    /// Approved adapter plan.
    pub adapter_plan_id: Sha256Digest,
    /// Resolved projections sorted by adapter and projection ID.
    pub projections: Vec<ResolvedProjection>,
    /// Creation time, excluded from identity.
    pub created_at: DateTime<Utc>,
}

/// One static or fixture conformance issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterConformanceIssue {
    /// Stable issue code.
    pub code: String,
    /// Issue severity.
    pub severity: Severity,
    /// Human-readable explanation.
    pub message: String,
    /// Optional fixture ID.
    pub fixture_id: Option<String>,
}

/// Deterministic conformance result for one exact package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterConformanceReport {
    /// Report schema.
    pub schema_version: String,
    /// Canonical conformance-report identity.
    pub id: Sha256Digest,
    /// Exact package hash.
    pub package_hash: Sha256Digest,
    /// Whether no error/critical issue occurred.
    pub passed: bool,
    /// Number of performed checks.
    pub checks: u64,
    /// Issues sorted deterministically.
    pub issues: Vec<AdapterConformanceIssue>,
    /// Creation time, excluded from identity.
    pub created_at: DateTime<Utc>,
}

/// Request to make a stage-output decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Run containing the stage.
    pub run_id: RunId,
    /// Stage to review.
    pub stage_id: StageId,
    /// Optimistic concurrency guard.
    pub expected_output_hash: Sha256Digest,
    /// Decision.
    pub decision: Decision,
    /// Reviewer.
    pub actor_id: ActorId,
    /// Explanation.
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_ids_accept_only_safe_characters() {
        assert!(ProjectId::new("ONCOVA-1.0").is_ok());
        assert!(ProjectId::new("bad/path").is_err());
        assert!(ProjectId::new("").is_err());
        assert!(serde_json::from_str::<AdapterId>(r#""pathex.v1""#).is_ok());
        assert!(serde_json::from_str::<AdapterId>(r#""../unsafe""#).is_err());
    }

    #[test]
    fn digest_is_normalized() {
        let upper = "A".repeat(64);
        let digest = Sha256Digest::new(upper).expect("valid digest");
        assert_eq!(digest.as_str(), "a".repeat(64));
        let encoded = format!(r#""{}""#, "B".repeat(64));
        assert_eq!(
            serde_json::from_str::<Sha256Digest>(&encoded)
                .expect("valid serialized digest")
                .as_str(),
            "b".repeat(64)
        );
        assert!(serde_json::from_str::<Sha256Digest>(r#""not-a-digest""#).is_err());
    }

    #[test]
    fn run_ids_round_trip() {
        let id = RunId::new();
        let parsed: RunId = id.to_string().parse().expect("UUID parses");
        assert_eq!(id, parsed);
    }
}
