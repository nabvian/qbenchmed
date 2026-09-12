//! Safe, deterministic, read-only project intake and immutable snapshots.

use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata, OpenOptions},
    io::{BufReader, Read},
    path::{Component, Path, PathBuf},
};

use chrono::Utc;
use flate2::read::GzDecoder;
use qbm_canonical::{CanonicalError, hash_value};
use qbm_domain::{
    DOMAIN_SCHEMA_VERSION, IntakePolicy, IntakeSourceKind, Inventory, InventoryDisposition,
    InventoryEntry, InventoryEntryKind, InventoryId, ProjectId, RunId, Sha256Digest,
    SnapshotArtifact, SourceSnapshot,
};
use qbm_store::{PlatformStore, StoreError};
use serde::Serialize;
use tar::Archive;
use thiserror::Error;
use zip::ZipArchive;

/// Intake and snapshot construction failures.
#[derive(Debug, Error)]
pub enum IntakeError {
    /// Filesystem access failed.
    #[error("intake filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    /// Artifact persistence or integrity verification failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Canonical identity generation failed.
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    /// ZIP decoding failed.
    #[error("ZIP archive is invalid or unsupported: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// The source is neither a directory nor a supported archive.
    #[error("unsupported intake source {0}; expected a directory, .zip, .tar, .tar.gz or .tgz")]
    UnsupportedSource(PathBuf),
    /// A relative source entry could not be represented safely and portably.
    #[error("unsafe or non-portable source entry path {0:?}")]
    UnsafePath(String),
    /// The inventory cannot be complete within the configured entry bound.
    #[error("source contains more than the configured maximum of {0} entries")]
    EntryLimit(u64),
    /// An archive member expanded to a size different from its declaration.
    #[error("archive entry {path:?} declared {declared} bytes but produced {actual}")]
    ArchiveSizeMismatch {
        /// Normalized archive path.
        path: String,
        /// Header-declared size.
        declared: u64,
        /// Bytes actually read.
        actual: u64,
    },
    /// A source file changed identity or metadata while it was being frozen.
    #[error("source file changed during intake: {0:?}; retry the scan")]
    SourceChanged(String),
    /// A blocked inventory cannot become an approved immutable snapshot.
    #[error("inventory {inventory_id} has {blocked_entries} blocked entries")]
    BlockedInventory {
        /// Inventory record.
        inventory_id: InventoryId,
        /// Safety-blocked entry count.
        blocked_entries: u64,
    },
    /// A persisted inventory violated its own included-file invariant.
    #[error("included file {path:?} in inventory {inventory_id} has no content digest")]
    MissingDigest {
        /// Inventory record.
        inventory_id: InventoryId,
        /// Invalid included path.
        path: String,
    },
}

/// Stateless intake service backed by the platform content-addressed store.
#[derive(Debug, Clone)]
pub struct IntakeEngine {
    store: PlatformStore,
}

#[derive(Debug, Serialize)]
struct InventoryIdentity<'a> {
    schema_version: &'static str,
    project_id: &'a ProjectId,
    source_kind: IntakeSourceKind,
    source_container_hash: Option<&'a Sha256Digest>,
    policy_hash: &'a Sha256Digest,
    entries: &'a [InventoryEntry],
}

#[derive(Debug, Serialize)]
struct SnapshotIdentity<'a> {
    schema_version: &'static str,
    inventory_hash: &'a Sha256Digest,
    policy_hash: &'a Sha256Digest,
    project_id: &'a ProjectId,
    artifacts: &'a [SnapshotArtifact],
    total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PolicyDecision {
    Include,
    ExcludeName,
    ExcludePrefix,
    OutsideIncludedPrefixes,
}

impl PolicyDecision {
    fn reason(self) -> Option<String> {
        match self {
            Self::Include => None,
            Self::ExcludeName => Some("excluded_name".to_owned()),
            Self::ExcludePrefix => Some("excluded_prefix".to_owned()),
            Self::OutsideIncludedPrefixes => Some("outside_included_prefixes".to_owned()),
        }
    }
}

impl IntakeEngine {
    /// Create an intake service using an existing platform store.
    #[must_use]
    pub fn new(store: PlatformStore) -> Self {
        Self { store }
    }

    /// Detect the source type, freeze included bytes, and build a deterministic inventory.
    pub fn scan(
        &self,
        run_id: RunId,
        project_id: ProjectId,
        source_path: &Path,
        policy: &IntakePolicy,
    ) -> Result<Inventory, IntakeError> {
        let canonical_source = source_path.canonicalize()?;
        let policy_hash = hash_value(policy)?;
        let (source_kind, source_container_hash, mut entries) = if canonical_source.is_dir() {
            (
                IntakeSourceKind::Directory,
                None,
                self.scan_directory(&canonical_source, policy)?,
            )
        } else if canonical_source.is_file() {
            let source_kind = detect_archive_kind(&canonical_source)?;
            let mut input = BufReader::new(open_regular_no_follow(&canonical_source)?);
            let before = input.get_ref().metadata()?;
            let container = self.store.put_reader(
                &mut input,
                canonical_source
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                Some(archive_media_type(source_kind).to_owned()),
            )?;
            let after = input.get_ref().metadata()?;
            if !same_file_state(&before, &after) || container.size_bytes != before.len() {
                return Err(IntakeError::SourceChanged(
                    canonical_source.to_string_lossy().into_owned(),
                ));
            }
            let frozen_path = self.store.artifact_path(&container.sha256)?;
            let entries =
                self.scan_archive(&frozen_path, source_kind, policy, container.size_bytes)?;
            (source_kind, Some(container.sha256), entries)
        } else {
            return Err(IntakeError::UnsupportedSource(canonical_source));
        };

        entries.sort_by(|left, right| {
            left.relative_path
                .cmp(&right.relative_path)
                .then_with(|| entry_kind_rank(left.kind).cmp(&entry_kind_rank(right.kind)))
                .then_with(|| left.sha256.cmp(&right.sha256))
        });
        let included_files = entries
            .iter()
            .filter(|entry| {
                entry.kind == InventoryEntryKind::File
                    && entry.disposition == InventoryDisposition::Included
            })
            .count() as u64;
        let excluded_entries = entries
            .iter()
            .filter(|entry| entry.disposition == InventoryDisposition::Excluded)
            .count() as u64;
        let blocked_entries = entries
            .iter()
            .filter(|entry| entry.disposition == InventoryDisposition::Blocked)
            .count() as u64;
        let total_included_bytes = entries
            .iter()
            .filter(|entry| {
                entry.kind == InventoryEntryKind::File
                    && entry.disposition == InventoryDisposition::Included
            })
            .map(|entry| entry.size_bytes)
            .sum();
        let inventory_hash = hash_value(&InventoryIdentity {
            schema_version: "qbm.inventory-identity/v1",
            project_id: &project_id,
            source_kind,
            source_container_hash: source_container_hash.as_ref(),
            policy_hash: &policy_hash,
            entries: &entries,
        })?;

        Ok(Inventory {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id: InventoryId::new(),
            run_id,
            project_id,
            source_kind,
            source_path: canonical_source,
            source_container_hash,
            policy_hash,
            inventory_hash,
            entries,
            included_files,
            excluded_entries,
            blocked_entries,
            total_included_bytes,
            created_at: Utc::now(),
        })
    }

    /// Build an immutable source snapshot from already-frozen inventory content.
    pub fn build_snapshot(&self, inventory: &Inventory) -> Result<SourceSnapshot, IntakeError> {
        if inventory.blocked_entries != 0 {
            return Err(IntakeError::BlockedInventory {
                inventory_id: inventory.id,
                blocked_entries: inventory.blocked_entries,
            });
        }
        let mut artifacts = Vec::new();
        for entry in inventory.entries.iter().filter(|entry| {
            entry.kind == InventoryEntryKind::File
                && entry.disposition == InventoryDisposition::Included
        }) {
            let sha256 = entry
                .sha256
                .clone()
                .ok_or_else(|| IntakeError::MissingDigest {
                    inventory_id: inventory.id,
                    path: entry.relative_path.clone(),
                })?;
            artifacts.push(SnapshotArtifact {
                relative_path: entry.relative_path.clone(),
                sha256,
                size_bytes: entry.size_bytes,
            });
        }
        for artifact in &artifacts {
            self.store.verify_artifact(&artifact.sha256)?;
        }
        let total_bytes = artifacts.iter().map(|artifact| artifact.size_bytes).sum();
        let id = hash_value(&SnapshotIdentity {
            schema_version: "qbm.source-snapshot-identity/v1",
            inventory_hash: &inventory.inventory_hash,
            policy_hash: &inventory.policy_hash,
            project_id: &inventory.project_id,
            artifacts: &artifacts,
            total_bytes,
        })?;
        Ok(SourceSnapshot {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id,
            inventory_hash: inventory.inventory_hash.clone(),
            policy_hash: inventory.policy_hash.clone(),
            project_id: inventory.project_id.clone(),
            artifacts,
            total_bytes,
            created_at: Utc::now(),
        })
    }

    #[allow(clippy::too_many_lines)]
    fn scan_directory(
        &self,
        root: &Path,
        policy: &IntakePolicy,
    ) -> Result<Vec<InventoryEntry>, IntakeError> {
        let mut entries = Vec::new();
        let mut stack = vec![(root.to_path_buf(), String::new())];
        let mut included_bytes = 0_u64;

        while let Some((directory, relative_directory)) = stack.pop() {
            let mut children = fs::read_dir(&directory)?.collect::<Result<Vec<_>, _>>()?;
            children.sort_by_key(std::fs::DirEntry::file_name);
            for child in children {
                enforce_entry_limit(entries.len(), policy.max_entries)?;
                let name = child
                    .file_name()
                    .into_string()
                    .map_err(|name| IntakeError::UnsafePath(name.to_string_lossy().into_owned()))?;
                let relative_path = if relative_directory.is_empty() {
                    normalize_relative_text(&name)?
                } else {
                    normalize_relative_text(&format!("{relative_directory}/{name}"))?
                };
                let depth = path_depth(&relative_path);
                let metadata = fs::symlink_metadata(child.path())?;
                let file_type = metadata.file_type();
                let kind = if file_type.is_file() {
                    InventoryEntryKind::File
                } else if file_type.is_dir() {
                    InventoryEntryKind::Directory
                } else if file_type.is_symlink() {
                    InventoryEntryKind::Symlink
                } else {
                    InventoryEntryKind::Special
                };

                if depth > policy.max_depth {
                    entries.push(blocked_entry(
                        relative_path,
                        kind,
                        metadata.len(),
                        "max_depth_exceeded",
                    ));
                    continue;
                }
                let decision = policy_decision(
                    &relative_path,
                    kind == InventoryEntryKind::Directory,
                    policy,
                );
                if decision != PolicyDecision::Include {
                    entries.push(InventoryEntry {
                        relative_path,
                        kind,
                        disposition: InventoryDisposition::Excluded,
                        size_bytes: if kind == InventoryEntryKind::File {
                            metadata.len()
                        } else {
                            0
                        },
                        sha256: None,
                        reason: decision.reason(),
                    });
                    continue;
                }
                match kind {
                    InventoryEntryKind::Directory => {
                        entries.push(included_container(relative_path.clone(), kind));
                        stack.push((child.path(), relative_path));
                    }
                    InventoryEntryKind::Symlink => entries.push(excluded_special(
                        relative_path,
                        kind,
                        "symlink_not_followed",
                    )),
                    InventoryEntryKind::Special => entries.push(excluded_special(
                        relative_path,
                        kind,
                        "special_file_not_read",
                    )),
                    InventoryEntryKind::File => {
                        let mut input = BufReader::new(open_regular_no_follow(&child.path())?);
                        let before = input.get_ref().metadata()?;
                        if !same_file_state(&metadata, &before) {
                            return Err(IntakeError::SourceChanged(relative_path));
                        }
                        let size = before.len();
                        if let Some(reason) = size_block_reason(size, included_bytes, policy) {
                            entries.push(blocked_entry(relative_path, kind, size, reason));
                            continue;
                        }
                        let artifact =
                            self.store
                                .put_reader(&mut input, Some(relative_path.clone()), None)?;
                        let after = input.get_ref().metadata()?;
                        if !same_file_state(&before, &after) {
                            return Err(IntakeError::SourceChanged(relative_path));
                        }
                        if artifact.size_bytes != size {
                            return Err(IntakeError::ArchiveSizeMismatch {
                                path: relative_path,
                                declared: size,
                                actual: artifact.size_bytes,
                            });
                        }
                        included_bytes = included_bytes.saturating_add(size);
                        entries.push(InventoryEntry {
                            relative_path,
                            kind,
                            disposition: InventoryDisposition::Included,
                            size_bytes: size,
                            sha256: Some(artifact.sha256),
                            reason: None,
                        });
                    }
                }
            }
        }
        Ok(entries)
    }

    fn scan_archive(
        &self,
        frozen_path: &Path,
        kind: IntakeSourceKind,
        policy: &IntakePolicy,
        compressed_size: u64,
    ) -> Result<Vec<InventoryEntry>, IntakeError> {
        match kind {
            IntakeSourceKind::ZipArchive => self.scan_zip(frozen_path, policy),
            IntakeSourceKind::TarArchive => {
                self.scan_tar(File::open(frozen_path)?, policy, compressed_size)
            }
            IntakeSourceKind::TarGzArchive => self.scan_tar(
                GzDecoder::new(BufReader::new(File::open(frozen_path)?)),
                policy,
                compressed_size,
            ),
            IntakeSourceKind::Directory => unreachable!("directory intake uses filesystem scan"),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn scan_zip(
        &self,
        path: &Path,
        policy: &IntakePolicy,
    ) -> Result<Vec<InventoryEntry>, IntakeError> {
        let mut archive = ZipArchive::new(File::open(path)?)?;
        let archive_entries = u64::try_from(archive.len()).unwrap_or(u64::MAX);
        if archive_entries > policy.max_entries {
            return Err(IntakeError::EntryLimit(policy.max_entries));
        }
        let mut entries = Vec::with_capacity(archive.len());
        let mut included_bytes = 0_u64;
        let mut observed_paths = BTreeSet::new();
        for index in 0..archive.len() {
            let mut member = archive.by_index(index)?;
            let raw_name = std::str::from_utf8(member.name_raw())
                .map_err(|_| IntakeError::UnsafePath("non-UTF-8 ZIP name".to_owned()))?;
            let relative_path = normalize_relative_text(raw_name)?;
            let is_directory = member.is_dir();
            let unix_kind = member.unix_mode().map(|mode| mode & 0o170_000);
            let kind = if is_directory {
                InventoryEntryKind::Directory
            } else if unix_kind == Some(0o120_000) {
                InventoryEntryKind::Symlink
            } else if unix_kind.is_none_or(|value| value == 0 || value == 0o100_000) {
                InventoryEntryKind::File
            } else {
                InventoryEntryKind::Special
            };
            if !observed_paths.insert(relative_path.clone()) {
                entries.push(blocked_entry(
                    relative_path,
                    kind,
                    member.size(),
                    "duplicate_path",
                ));
                continue;
            }
            if path_depth(&relative_path) > policy.max_depth {
                entries.push(blocked_entry(
                    relative_path,
                    kind,
                    member.size(),
                    "max_depth_exceeded",
                ));
                continue;
            }
            let decision = policy_decision(&relative_path, is_directory, policy);
            if decision != PolicyDecision::Include {
                entries.push(InventoryEntry {
                    relative_path,
                    kind,
                    disposition: InventoryDisposition::Excluded,
                    size_bytes: if kind == InventoryEntryKind::File {
                        member.size()
                    } else {
                        0
                    },
                    sha256: None,
                    reason: decision.reason(),
                });
                continue;
            }
            if is_directory {
                entries.push(included_container(relative_path, kind));
                continue;
            }
            if kind != InventoryEntryKind::File {
                let reason = if kind == InventoryEntryKind::Symlink {
                    "symlink_not_followed"
                } else {
                    "special_archive_entry_not_read"
                };
                entries.push(excluded_special(relative_path, kind, reason));
                continue;
            }
            let declared_size = member.size();
            if compression_ratio_exceeded(
                declared_size,
                member.compressed_size(),
                policy.max_compression_ratio,
            ) {
                entries.push(blocked_entry(
                    relative_path,
                    kind,
                    declared_size,
                    "max_compression_ratio_exceeded",
                ));
                continue;
            }
            if let Some(reason) = size_block_reason(declared_size, included_bytes, policy) {
                entries.push(blocked_entry(relative_path, kind, declared_size, reason));
                continue;
            }
            let read_bound = declared_size.saturating_add(1);
            let artifact = self.store.put_reader(
                (&mut member).take(read_bound),
                Some(relative_path.clone()),
                None,
            )?;
            if artifact.size_bytes != declared_size {
                return Err(IntakeError::ArchiveSizeMismatch {
                    path: relative_path,
                    declared: declared_size,
                    actual: artifact.size_bytes,
                });
            }
            included_bytes = included_bytes.saturating_add(declared_size);
            entries.push(InventoryEntry {
                relative_path,
                kind,
                disposition: InventoryDisposition::Included,
                size_bytes: declared_size,
                sha256: Some(artifact.sha256),
                reason: None,
            });
        }
        Ok(entries)
    }

    #[allow(clippy::too_many_lines)]
    fn scan_tar(
        &self,
        reader: impl Read,
        policy: &IntakePolicy,
        compressed_size: u64,
    ) -> Result<Vec<InventoryEntry>, IntakeError> {
        let mut archive = Archive::new(reader);
        let mut entries = Vec::new();
        let mut included_bytes = 0_u64;
        let mut declared_archive_bytes = 0_u64;
        let mut observed_paths = BTreeSet::new();
        for member in archive.entries()? {
            enforce_entry_limit(entries.len(), policy.max_entries)?;
            let mut member = member?;
            let member_path = member.path()?;
            let relative_path = normalize_relative_path(&member_path)?;
            let entry_type = member.header().entry_type();
            let kind = if entry_type.is_file() {
                InventoryEntryKind::File
            } else if entry_type.is_dir() {
                InventoryEntryKind::Directory
            } else if entry_type.is_symlink() {
                InventoryEntryKind::Symlink
            } else {
                InventoryEntryKind::Special
            };
            let declared_size = member.header().size()?;
            declared_archive_bytes = declared_archive_bytes.saturating_add(declared_size);
            if compression_ratio_exceeded(
                declared_archive_bytes,
                compressed_size,
                policy.max_compression_ratio,
            ) {
                entries.push(blocked_entry(
                    relative_path,
                    kind,
                    declared_size,
                    "max_compression_ratio_exceeded",
                ));
                continue;
            }
            if !observed_paths.insert(relative_path.clone()) {
                entries.push(blocked_entry(
                    relative_path,
                    kind,
                    declared_size,
                    "duplicate_path",
                ));
                continue;
            }
            if path_depth(&relative_path) > policy.max_depth {
                entries.push(blocked_entry(
                    relative_path,
                    kind,
                    declared_size,
                    "max_depth_exceeded",
                ));
                continue;
            }
            let decision = policy_decision(
                &relative_path,
                kind == InventoryEntryKind::Directory,
                policy,
            );
            if decision != PolicyDecision::Include {
                entries.push(InventoryEntry {
                    relative_path,
                    kind,
                    disposition: InventoryDisposition::Excluded,
                    size_bytes: if kind == InventoryEntryKind::File {
                        declared_size
                    } else {
                        0
                    },
                    sha256: None,
                    reason: decision.reason(),
                });
                continue;
            }
            match kind {
                InventoryEntryKind::Directory => {
                    entries.push(included_container(relative_path, kind));
                }
                InventoryEntryKind::Symlink => entries.push(excluded_special(
                    relative_path,
                    kind,
                    "symlink_not_followed",
                )),
                InventoryEntryKind::Special => entries.push(excluded_special(
                    relative_path,
                    kind,
                    "special_archive_entry_not_read",
                )),
                InventoryEntryKind::File => {
                    if let Some(reason) = size_block_reason(declared_size, included_bytes, policy) {
                        entries.push(blocked_entry(relative_path, kind, declared_size, reason));
                        continue;
                    }
                    let artifact = self.store.put_reader(
                        (&mut member).take(declared_size.saturating_add(1)),
                        Some(relative_path.clone()),
                        None,
                    )?;
                    if artifact.size_bytes != declared_size {
                        return Err(IntakeError::ArchiveSizeMismatch {
                            path: relative_path,
                            declared: declared_size,
                            actual: artifact.size_bytes,
                        });
                    }
                    included_bytes = included_bytes.saturating_add(declared_size);
                    entries.push(InventoryEntry {
                        relative_path,
                        kind,
                        disposition: InventoryDisposition::Included,
                        size_bytes: declared_size,
                        sha256: Some(artifact.sha256),
                        reason: None,
                    });
                }
            }
        }
        Ok(entries)
    }
}

fn detect_archive_kind(path: &Path) -> Result<IntakeSourceKind, IntakeError> {
    let extension = path.extension().and_then(|value| value.to_str());
    let stem_extension = path
        .file_stem()
        .map(Path::new)
        .and_then(Path::extension)
        .and_then(|value| value.to_str());
    if extension.is_some_and(|value| value.eq_ignore_ascii_case("tgz"))
        || (extension.is_some_and(|value| value.eq_ignore_ascii_case("gz"))
            && stem_extension.is_some_and(|value| value.eq_ignore_ascii_case("tar")))
    {
        Ok(IntakeSourceKind::TarGzArchive)
    } else if extension.is_some_and(|value| value.eq_ignore_ascii_case("tar")) {
        Ok(IntakeSourceKind::TarArchive)
    } else if extension.is_some_and(|value| value.eq_ignore_ascii_case("zip")) {
        Ok(IntakeSourceKind::ZipArchive)
    } else {
        Err(IntakeError::UnsupportedSource(path.to_path_buf()))
    }
}

#[cfg(unix)]
fn open_regular_no_follow(path: &Path) -> Result<File, std::io::Error> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_regular_no_follow(path: &Path) -> Result<File, std::io::Error> {
    OpenOptions::new().read(true).open(path)
}

fn same_file_state(before: &Metadata, after: &Metadata) -> bool {
    if !before.is_file()
        || !after.is_file()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return false;
    }
    same_platform_file_identity(before, after)
}

#[cfg(unix)]
fn same_platform_file_identity(before: &Metadata, after: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    before.dev() == after.dev() && before.ino() == after.ino()
}

#[cfg(not(unix))]
fn same_platform_file_identity(_before: &Metadata, _after: &Metadata) -> bool {
    true
}

const fn archive_media_type(kind: IntakeSourceKind) -> &'static str {
    match kind {
        IntakeSourceKind::ZipArchive => "application/zip",
        IntakeSourceKind::TarArchive => "application/x-tar",
        IntakeSourceKind::TarGzArchive => "application/gzip",
        IntakeSourceKind::Directory => "application/octet-stream",
    }
}

fn normalize_relative_path(path: &Path) -> Result<String, IntakeError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| IntakeError::UnsafePath(value.to_string_lossy().into_owned()))?
                    .to_owned(),
            ),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(IntakeError::UnsafePath(path.to_string_lossy().into_owned()));
            }
        }
    }
    normalize_relative_text(&parts.join("/"))
}

fn normalize_relative_text(value: &str) -> Result<String, IntakeError> {
    let normalized = value.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains('\0') {
        return Err(IntakeError::UnsafePath(value.to_owned()));
    }
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(IntakeError::UnsafePath(value.to_owned()));
        }
        if parts.is_empty()
            && part.len() == 2
            && part.as_bytes()[0].is_ascii_alphabetic()
            && part.ends_with(':')
        {
            return Err(IntakeError::UnsafePath(value.to_owned()));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(IntakeError::UnsafePath(value.to_owned()));
    }
    Ok(parts.join("/"))
}

fn path_depth(path: &str) -> u32 {
    u32::try_from(path.split('/').count()).unwrap_or(u32::MAX)
}

fn policy_decision(path: &str, is_directory: bool, policy: &IntakePolicy) -> PolicyDecision {
    let components = path.split('/').collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| policy.excluded_names.contains(*component))
    {
        return PolicyDecision::ExcludeName;
    }
    if policy
        .excluded_prefixes
        .iter()
        .any(|prefix| path_matches_prefix(path, prefix))
    {
        return PolicyDecision::ExcludePrefix;
    }
    if policy.included_prefixes.is_empty()
        || policy.included_prefixes.iter().any(|prefix| {
            path_matches_prefix(path, prefix) || (is_directory && path_matches_prefix(prefix, path))
        })
    {
        PolicyDecision::Include
    } else {
        PolicyDecision::OutsideIncludedPrefixes
    }
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_matches('/');
    !prefix.is_empty()
        && (path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('/')))
}

fn size_block_reason(
    size: u64,
    included_bytes: u64,
    policy: &IntakePolicy,
) -> Option<&'static str> {
    if size > policy.max_single_file_bytes {
        Some("max_single_file_bytes_exceeded")
    } else if included_bytes.saturating_add(size) > policy.max_total_bytes {
        Some("max_total_bytes_exceeded")
    } else {
        None
    }
}

fn compression_ratio_exceeded(uncompressed: u64, compressed: u64, maximum: u64) -> bool {
    uncompressed != 0 && (compressed == 0 || uncompressed > compressed.saturating_mul(maximum))
}

fn enforce_entry_limit(observed: usize, maximum: u64) -> Result<(), IntakeError> {
    if u64::try_from(observed).unwrap_or(u64::MAX) >= maximum {
        Err(IntakeError::EntryLimit(maximum))
    } else {
        Ok(())
    }
}

fn included_container(relative_path: String, kind: InventoryEntryKind) -> InventoryEntry {
    InventoryEntry {
        relative_path,
        kind,
        disposition: InventoryDisposition::Included,
        size_bytes: 0,
        sha256: None,
        reason: None,
    }
}

fn excluded_special(
    relative_path: String,
    kind: InventoryEntryKind,
    reason: &str,
) -> InventoryEntry {
    InventoryEntry {
        relative_path,
        kind,
        disposition: InventoryDisposition::Excluded,
        size_bytes: 0,
        sha256: None,
        reason: Some(reason.to_owned()),
    }
}

fn blocked_entry(
    relative_path: String,
    kind: InventoryEntryKind,
    size_bytes: u64,
    reason: &str,
) -> InventoryEntry {
    InventoryEntry {
        relative_path,
        kind,
        disposition: InventoryDisposition::Blocked,
        size_bytes,
        sha256: None,
        reason: Some(reason.to_owned()),
    }
}

const fn entry_kind_rank(kind: InventoryEntryKind) -> u8 {
    match kind {
        InventoryEntryKind::Directory => 0,
        InventoryEntryKind::File => 1,
        InventoryEntryKind::Symlink => 2,
        InventoryEntryKind::Special => 3,
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use qbm_domain::{ProjectId, RunId};
    use tempfile::TempDir;

    use super::*;

    fn engine() -> (TempDir, IntakeEngine) {
        let data = TempDir::new().unwrap();
        let store = PlatformStore::open(data.path()).unwrap();
        (data, IntakeEngine::new(store))
    }

    #[test]
    fn directory_inventory_is_deterministic_and_freezes_files() {
        let (_data, engine) = engine();
        let source = TempDir::new().unwrap();
        fs::create_dir(source.path().join("src")).unwrap();
        fs::write(source.path().join("src/main.rs"), b"fn main() {}\n").unwrap();
        fs::write(source.path().join("README.md"), b"demo\n").unwrap();
        let project = ProjectId::new("demo").unwrap();
        let first = engine
            .scan(
                RunId::new(),
                project.clone(),
                source.path(),
                &IntakePolicy::default(),
            )
            .unwrap();
        let second = engine
            .scan(
                RunId::new(),
                project,
                source.path(),
                &IntakePolicy::default(),
            )
            .unwrap();
        assert_eq!(first.inventory_hash, second.inventory_hash);
        assert_eq!(first.included_files, 2);
        assert_eq!(first.blocked_entries, 0);
        assert!(
            first
                .entries
                .iter()
                .filter_map(|entry| entry.sha256.as_ref())
                .count()
                == 2
        );
    }

    #[test]
    fn default_policy_excludes_build_and_vcs_content() {
        let (_data, engine) = engine();
        let source = TempDir::new().unwrap();
        fs::create_dir(source.path().join(".git")).unwrap();
        fs::create_dir(source.path().join("target")).unwrap();
        fs::write(source.path().join(".git/config"), b"secret").unwrap();
        fs::write(source.path().join("target/output"), b"binary").unwrap();
        fs::write(source.path().join("keep.txt"), b"keep").unwrap();
        let inventory = engine
            .scan(
                RunId::new(),
                ProjectId::new("demo").unwrap(),
                source.path(),
                &IntakePolicy::default(),
            )
            .unwrap();
        assert_eq!(inventory.included_files, 1);
        assert_eq!(inventory.excluded_entries, 2);
        assert!(
            !inventory
                .entries
                .iter()
                .any(|entry| entry.relative_path == ".git/config")
        );
    }

    #[test]
    fn oversized_file_blocks_snapshot() {
        let (_data, engine) = engine();
        let source = TempDir::new().unwrap();
        fs::write(source.path().join("large.bin"), b"12345").unwrap();
        let policy = IntakePolicy {
            max_single_file_bytes: 4,
            ..IntakePolicy::default()
        };
        let inventory = engine
            .scan(
                RunId::new(),
                ProjectId::new("demo").unwrap(),
                source.path(),
                &policy,
            )
            .unwrap();
        assert_eq!(inventory.blocked_entries, 1);
        assert!(matches!(
            engine.build_snapshot(&inventory),
            Err(IntakeError::BlockedInventory { .. })
        ));
    }

    #[test]
    fn tar_archive_is_read_without_extraction() {
        let (_data, engine) = engine();
        let source = TempDir::new().unwrap();
        let archive_path = source.path().join("project.tar");
        let mut builder = tar::Builder::new(File::create(&archive_path).unwrap());
        let bytes = b"hello archive\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "src/data.txt", &bytes[..])
            .unwrap();
        builder.finish().unwrap();
        let inventory = engine
            .scan(
                RunId::new(),
                ProjectId::new("archive").unwrap(),
                &archive_path,
                &IntakePolicy::default(),
            )
            .unwrap();
        assert_eq!(inventory.source_kind, IntakeSourceKind::TarArchive);
        assert_eq!(inventory.included_files, 1);
        assert!(!source.path().join("src").is_dir());
    }

    #[test]
    fn zip_archive_is_streamed_into_content_store() {
        let (_data, engine) = engine();
        let source = TempDir::new().unwrap();
        let archive_path = source.path().join("project.zip");
        create_zip(&archive_path);
        let inventory = engine
            .scan(
                RunId::new(),
                ProjectId::new("zip-project").unwrap(),
                &archive_path,
                &IntakePolicy::default(),
            )
            .unwrap();
        assert_eq!(inventory.source_kind, IntakeSourceKind::ZipArchive);
        assert_eq!(inventory.included_files, 1);
        assert_eq!(inventory.blocked_entries, 0);
        assert!(!source.path().join("src").exists());
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlink_is_recorded_but_never_followed() {
        use std::os::unix::fs::symlink;

        let (_data, engine) = engine();
        let source = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.txt"), b"do not follow").unwrap();
        symlink(
            outside.path().join("secret.txt"),
            source.path().join("link.txt"),
        )
        .unwrap();
        let inventory = engine
            .scan(
                RunId::new(),
                ProjectId::new("links").unwrap(),
                source.path(),
                &IntakePolicy::default(),
            )
            .unwrap();
        assert_eq!(inventory.included_files, 0);
        assert_eq!(inventory.excluded_entries, 1);
        assert_eq!(inventory.entries[0].kind, InventoryEntryKind::Symlink);
        assert_eq!(
            inventory.entries[0].reason.as_deref(),
            Some("symlink_not_followed")
        );
    }

    #[test]
    fn normalization_rejects_archive_traversal() {
        assert!(normalize_relative_text("../../etc/passwd").is_err());
        assert!(normalize_relative_text("/absolute/file").is_err());
        assert!(normalize_relative_text("C:\\Windows\\file").is_err());
    }

    #[test]
    fn snapshot_identity_is_repeatable() {
        let (_data, engine) = engine();
        let source = TempDir::new().unwrap();
        fs::write(source.path().join("input.yaml"), b"enabled: true\n").unwrap();
        let inventory = engine
            .scan(
                RunId::new(),
                ProjectId::new("demo").unwrap(),
                source.path(),
                &IntakePolicy::default(),
            )
            .unwrap();
        let first = engine.build_snapshot(&inventory).unwrap();
        let second = engine.build_snapshot(&inventory).unwrap();
        assert_eq!(first.id, second.id);
    }

    fn create_zip(path: &Path) {
        let file = File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("src/main.rs", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"fn main() {}\n").unwrap();
        archive.finish().unwrap();
    }
}
