use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

use axum::extract::Multipart;
use qbm_canonical::hash_value;
use qbm_domain::{RunMode, Sha256Digest, SourceAcquisitionKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::{error::ApiError, github::GithubClient};

#[derive(Debug)]
pub(crate) struct AcquiredSource {
    staging: StagingDirectory,
    source_relative_path: PathBuf,
    pub kind: SourceAcquisitionKind,
    pub source_locator: String,
    pub repository_id: Option<u64>,
    pub requested_revision: Option<String>,
    pub resolved_revision: Option<String>,
    pub retrieval_url: Option<String>,
    pub provider_api_version: Option<String>,
    pub content_sha256: Sha256Digest,
    pub total_bytes: u64,
    pub suggested_project_name: String,
}

#[derive(Debug)]
pub(crate) struct ManagedSource {
    pub source_path: PathBuf,
}

#[derive(Debug)]
struct StagingDirectory {
    path: PathBuf,
    committed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderManifest {
    paths: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FolderFileIdentity {
    relative_path: String,
    sha256: Sha256Digest,
    size_bytes: u64,
}

impl StagingDirectory {
    fn create(data_directory: &Path) -> Result<Self, ApiError> {
        let staging_root = data_directory.join("imports").join(".staging");
        fs::create_dir_all(&staging_root)
            .map_err(|_| ApiError::internal("A managed upload area could not be created."))?;
        let path = staging_root.join(Uuid::new_v4().to_string());
        fs::create_dir(&path)
            .map_err(|_| ApiError::internal("A managed upload could not be staged."))?;
        Ok(Self {
            path,
            committed: false,
        })
    }

    fn commit(mut self, project_id: &str) -> Result<PathBuf, ApiError> {
        let imports_root = self
            .path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| ApiError::internal("The managed upload path was invalid."))?;
        let destination = imports_root.join(project_id);
        fs::rename(&self.path, &destination)
            .map_err(|_| ApiError::internal("The managed upload could not be committed."))?;
        self.committed = true;
        destination
            .canonicalize()
            .map_err(|_| ApiError::internal("The managed upload could not be reopened."))
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

impl AcquiredSource {
    pub(crate) fn commit(self, project_id: &str) -> Result<ManagedSource, ApiError> {
        let source_relative_path = self.source_relative_path.clone();
        let root = self.staging.commit(project_id)?;
        let source_path = if source_relative_path.as_os_str().is_empty() {
            root
        } else {
            root.join(source_relative_path)
                .canonicalize()
                .map_err(|_| ApiError::internal("The imported source could not be reopened."))?
        };
        Ok(ManagedSource { source_path })
    }
}

#[allow(clippy::too_many_lines)] // One pass over the upload's fields; splitting it would scatter the bounds.
pub(crate) async fn acquire_archive(
    data_directory: &Path,
    mut multipart: Multipart,
    maximum_bytes: u64,
) -> Result<(AcquiredSource, Option<String>, RunMode), ApiError> {
    let staging = StagingDirectory::create(data_directory)?;
    let mut project_name = None;
    let mut run_mode = RunMode::Governed;
    let mut source = None;

    while let Some(mut field) = multipart.next_field().await.map_err(|_| {
        ApiError::bad_request("invalid_multipart", "The upload form could not be read.")
    })? {
        match field.name() {
            Some("project_name") => {
                project_name = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| {
                            ApiError::bad_request(
                                "invalid_project_name",
                                "The project name could not be read.",
                            )
                        })?
                        .trim()
                        .to_owned(),
                );
            }
            Some("run_mode") => {
                let raw = field.text().await.map_err(|_| {
                    ApiError::bad_request("invalid_run_mode", "The run mode could not be read.")
                })?;
                run_mode = parse_browser_run_mode(raw.trim())?;
            }
            Some("source") => {
                if source.is_some() {
                    return Err(ApiError::bad_request(
                        "multiple_sources",
                        "Upload exactly one archive at a time.",
                    ));
                }
                let original_name = safe_upload_name(field.file_name().unwrap_or("source.zip"));
                let extension = supported_archive_suffix(&original_name).ok_or_else(|| {
                    ApiError::bad_request(
                        "unsupported_source",
                        "Choose a .zip, .tar, .tar.gz, or .tgz archive.",
                    )
                })?;
                let source_relative_path = PathBuf::from(format!("source{extension}"));
                let destination = staging.path.join(&source_relative_path);
                let mut output = tokio::fs::File::create(&destination)
                    .await
                    .map_err(|_| ApiError::internal("The archive could not be staged."))?;
                let mut hasher = Sha256::new();
                let mut total = 0_u64;
                while let Some(chunk) = field.chunk().await.map_err(|_| {
                    ApiError::bad_request("invalid_upload", "The archive upload was interrupted.")
                })? {
                    total = total.saturating_add(chunk.len() as u64);
                    if total > maximum_bytes {
                        return Err(ApiError::too_large(format!(
                            "The archive exceeds the {} MiB upload limit.",
                            maximum_bytes / 1024 / 1024
                        )));
                    }
                    hasher.update(&chunk);
                    output
                        .write_all(&chunk)
                        .await
                        .map_err(|_| ApiError::internal("The archive could not be staged."))?;
                }
                if total == 0 {
                    return Err(ApiError::bad_request(
                        "empty_source",
                        "The selected archive is empty.",
                    ));
                }
                output
                    .flush()
                    .await
                    .map_err(|_| ApiError::internal("The archive could not be committed."))?;
                let content_sha256 = Sha256Digest::new(format!("{:x}", hasher.finalize()))
                    .map_err(|_| ApiError::internal("The upload hash was invalid."))?;
                let suggested_project_name = display_stem(&original_name);
                source = Some(AcquiredSource {
                    staging,
                    source_relative_path,
                    kind: SourceAcquisitionKind::UploadedArchive,
                    source_locator: original_name,
                    repository_id: None,
                    requested_revision: None,
                    resolved_revision: None,
                    retrieval_url: None,
                    provider_api_version: None,
                    content_sha256,
                    total_bytes: total,
                    suggested_project_name,
                });
                break;
            }
            _ => {}
        }
    }

    let source = source.ok_or_else(|| {
        ApiError::bad_request(
            "missing_source",
            "Choose an archive before starting the audit.",
        )
    })?;
    Ok((
        source,
        project_name.filter(|name| !name.is_empty()),
        run_mode,
    ))
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn acquire_folder(
    data_directory: &Path,
    mut multipart: Multipart,
    maximum_bytes: u64,
    maximum_files: usize,
    maximum_single_file_bytes: u64,
) -> Result<(AcquiredSource, Option<String>, RunMode), ApiError> {
    let staging = StagingDirectory::create(data_directory)?;
    let mut project_name = None;
    let mut run_mode = RunMode::Governed;
    let mut paths = None;
    let mut identities = Vec::new();
    let mut total = 0_u64;

    while let Some(mut field) = multipart.next_field().await.map_err(|_| {
        ApiError::bad_request(
            "invalid_multipart",
            "The folder upload form could not be read.",
        )
    })? {
        match field.name() {
            Some("project_name") => {
                project_name = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| {
                            ApiError::bad_request(
                                "invalid_project_name",
                                "The project name could not be read.",
                            )
                        })?
                        .trim()
                        .to_owned(),
                );
            }
            Some("run_mode") => {
                let raw = field.text().await.map_err(|_| {
                    ApiError::bad_request("invalid_run_mode", "The run mode could not be read.")
                })?;
                run_mode = parse_browser_run_mode(raw.trim())?;
            }
            Some("manifest") => {
                if paths.is_some() {
                    return Err(ApiError::bad_request(
                        "duplicate_manifest",
                        "The folder upload contained more than one manifest.",
                    ));
                }
                let text = field.text().await.map_err(|_| {
                    ApiError::bad_request(
                        "invalid_folder_manifest",
                        "The folder file list could not be read.",
                    )
                })?;
                let manifest: FolderManifest = serde_json::from_str(&text).map_err(|_| {
                    ApiError::bad_request(
                        "invalid_folder_manifest",
                        "The folder file list was invalid.",
                    )
                })?;
                if manifest.paths.is_empty() || manifest.paths.len() > maximum_files {
                    return Err(ApiError::bad_request(
                        "folder_file_limit",
                        format!("Choose a non-empty folder with at most {maximum_files} files."),
                    ));
                }
                let normalized = manifest
                    .paths
                    .iter()
                    .map(|path| normalize_browser_path(path))
                    .collect::<Result<Vec<_>, _>>()?;
                let unique = normalized.iter().collect::<BTreeSet<_>>();
                if unique.len() != normalized.len() {
                    return Err(ApiError::bad_request(
                        "duplicate_folder_path",
                        "The folder contains duplicate normalized paths.",
                    ));
                }
                paths = Some(normalized);
            }
            Some("file") => {
                let paths = paths.as_ref().ok_or_else(|| {
                    ApiError::bad_request(
                        "manifest_required_first",
                        "The folder file list must be sent before its files.",
                    )
                })?;
                let relative_path = paths.get(identities.len()).ok_or_else(|| {
                    ApiError::bad_request(
                        "folder_manifest_mismatch",
                        "The folder contains more files than its manifest.",
                    )
                })?;
                let destination = staging.path.join(relative_path);
                if let Some(parent) = destination.parent() {
                    tokio::fs::create_dir_all(parent).await.map_err(|_| {
                        ApiError::internal("A folder-upload directory could not be staged.")
                    })?;
                }
                let mut output = tokio::fs::File::create(&destination)
                    .await
                    .map_err(|_| ApiError::internal("A folder file could not be staged."))?;
                let mut file_bytes = 0_u64;
                let mut hasher = Sha256::new();
                while let Some(chunk) = field.chunk().await.map_err(|_| {
                    ApiError::bad_request("invalid_upload", "The folder upload was interrupted.")
                })? {
                    file_bytes = file_bytes.saturating_add(chunk.len() as u64);
                    total = total.saturating_add(chunk.len() as u64);
                    if file_bytes > maximum_single_file_bytes || total > maximum_bytes {
                        return Err(ApiError::too_large(
                            "The folder exceeds the local upload safety limits.",
                        ));
                    }
                    hasher.update(&chunk);
                    output
                        .write_all(&chunk)
                        .await
                        .map_err(|_| ApiError::internal("A folder file could not be staged."))?;
                }
                output
                    .flush()
                    .await
                    .map_err(|_| ApiError::internal("A folder file could not be committed."))?;
                identities.push(FolderFileIdentity {
                    relative_path: relative_path.clone(),
                    sha256: Sha256Digest::new(format!("{:x}", hasher.finalize()))
                        .map_err(|_| ApiError::internal("A folder file hash was invalid."))?,
                    size_bytes: file_bytes,
                });
            }
            _ => {}
        }
    }

    let paths = paths.ok_or_else(|| {
        ApiError::bad_request(
            "missing_folder_manifest",
            "Choose a folder before starting the audit.",
        )
    })?;
    if identities.len() != paths.len() {
        return Err(ApiError::bad_request(
            "folder_manifest_mismatch",
            "The folder file count did not match its manifest.",
        ));
    }
    identities.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let content_sha256 = hash_value(&identities)
        .map_err(|_| ApiError::internal("The folder upload identity could not be created."))?;
    let root_name = paths
        .first()
        .and_then(|path| path.split('/').next())
        .unwrap_or("Uploaded folder")
        .to_owned();
    Ok((
        AcquiredSource {
            staging,
            source_relative_path: PathBuf::new(),
            kind: SourceAcquisitionKind::UploadedFolder,
            source_locator: root_name.clone(),
            repository_id: None,
            requested_revision: None,
            resolved_revision: None,
            retrieval_url: None,
            provider_api_version: None,
            content_sha256,
            total_bytes: total,
            suggested_project_name: root_name,
        },
        project_name.filter(|name| !name.is_empty()),
        run_mode,
    ))
}

pub(crate) async fn acquire_github(
    data_directory: &Path,
    client: &GithubClient,
    repository_url: &str,
    reference: Option<&str>,
    maximum_bytes: u64,
) -> Result<AcquiredSource, ApiError> {
    let staging = StagingDirectory::create(data_directory)?;
    let source_relative_path = PathBuf::from("source.zip");
    let destination = staging.path.join(&source_relative_path);
    let download = client
        .download(repository_url, reference, &destination, maximum_bytes)
        .await?;
    Ok(AcquiredSource {
        staging,
        source_relative_path,
        kind: SourceAcquisitionKind::PublicGithub,
        source_locator: download.canonical_url,
        repository_id: Some(download.repository_id),
        requested_revision: Some(download.requested_revision),
        resolved_revision: Some(download.resolved_revision),
        retrieval_url: Some(download.retrieval_url),
        provider_api_version: Some(crate::github::GITHUB_API_VERSION.to_owned()),
        content_sha256: download.archive_sha256,
        total_bytes: download.archive_bytes,
        suggested_project_name: download.suggested_name,
    })
}

fn safe_upload_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("source.zip")
        .chars()
        .filter(|character| !character.is_control())
        .take(200)
        .collect()
}

fn supported_archive_suffix(name: &str) -> Option<&'static str> {
    let lowercase = name.to_ascii_lowercase();
    if lowercase.ends_with(".tar.gz") {
        Some(".tar.gz")
    } else {
        Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| {
                if extension.eq_ignore_ascii_case("tgz") {
                    Some(".tgz")
                } else if extension.eq_ignore_ascii_case("zip") {
                    Some(".zip")
                } else if extension.eq_ignore_ascii_case("tar") {
                    Some(".tar")
                } else {
                    None
                }
            })
    }
}

fn display_stem(name: &str) -> String {
    let suffix = supported_archive_suffix(name).unwrap_or("");
    name.get(..name.len().saturating_sub(suffix.len()))
        .filter(|value| !value.is_empty())
        .unwrap_or("Uploaded project")
        .to_owned()
}

fn normalize_browser_path(value: &str) -> Result<String, ApiError> {
    if value.is_empty()
        || value.len() > 4096
        || value.starts_with('/')
        || value.contains("//")
        || value.contains(['\\', '\0'])
        || value.chars().any(char::is_control)
    {
        return Err(unsafe_folder_path());
    }
    let path = Path::new(value);
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(unsafe_folder_path)?;
                if part.is_empty()
                    || part.len() > 255
                    || matches!(part, "." | "..")
                    || part.ends_with(':')
                {
                    return Err(unsafe_folder_path());
                }
                parts.push(part);
            }
            _ => return Err(unsafe_folder_path()),
        }
    }
    if parts.is_empty() {
        return Err(unsafe_folder_path());
    }
    Ok(parts.join("/"))
}

fn unsafe_folder_path() -> ApiError {
    ApiError::bad_request(
        "unsafe_folder_path",
        "The folder contains an unsafe or non-portable relative path.",
    )
}

/// Run modes the browser may ask for.
///
/// Only two of the platform's modes make sense from a browser: review every
/// checkpoint, or accept them all by policy. The rest are internal audit modes
/// and are refused here rather than silently mapped onto one of these.
fn parse_browser_run_mode(value: &str) -> Result<RunMode, ApiError> {
    match value {
        "" | "governed" => Ok(RunMode::Governed),
        "express" => Ok(RunMode::Express),
        _ => Err(ApiError::bad_request(
            "unsupported_run_mode",
            "Choose either the governed or the express run mode.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_paths_are_relative_portable_and_normalized() {
        assert_eq!(
            normalize_browser_path("project/rules/config.yaml").unwrap(),
            "project/rules/config.yaml"
        );
        for rejected in [
            "../outside",
            "/absolute",
            "C:\\Windows\\file",
            "project/../outside",
            "project//file",
            "project/C:/file",
        ] {
            assert!(
                normalize_browser_path(rejected).is_err(),
                "accepted {rejected}"
            );
        }
    }

    #[test]
    fn archive_suffix_policy_is_explicit() {
        assert_eq!(supported_archive_suffix("source.ZIP"), Some(".zip"));
        assert_eq!(supported_archive_suffix("source.tar.gz"), Some(".tar.gz"));
        assert_eq!(supported_archive_suffix("source.rar"), None);
    }
}
