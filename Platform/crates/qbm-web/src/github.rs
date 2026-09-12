use std::{path::Path, time::Duration};

use futures_util::StreamExt;
use reqwest::{
    Client, Response, StatusCode, Url,
    header::{ACCEPT, ACCEPT_ENCODING, LOCATION},
    redirect::Policy,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::ApiError;

pub(crate) const GITHUB_API_VERSION: &str = "2026-03-10";
const MAX_API_BYTES: u64 = 1024 * 1024;
const MAX_SHA_BYTES: u64 = 128;
const MAX_URL_BYTES: usize = 2048;
const MAX_REF_BYTES: usize = 255;

#[derive(Debug, Clone)]
pub(crate) struct GithubClient {
    client: Client,
}

#[derive(Debug)]
pub(crate) struct GithubDownload {
    pub canonical_url: String,
    pub repository_id: u64,
    pub requested_revision: String,
    pub resolved_revision: String,
    pub retrieval_url: String,
    pub archive_sha256: qbm_domain::Sha256Digest,
    pub archive_bytes: u64,
    pub suggested_name: String,
}

#[derive(Debug)]
struct RepositoryLocator {
    owner: String,
    repository: String,
}

#[derive(Debug, Deserialize)]
struct RepositoryMetadata {
    id: u64,
    full_name: String,
    private: bool,
    visibility: Option<String>,
    default_branch: String,
}

impl GithubClient {
    pub(crate) fn new() -> Result<Self, ApiError> {
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(600))
            .user_agent(concat!("qbenchmed-platform/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| ApiError::internal("The GitHub client could not be initialized."))?;
        Ok(Self { client })
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) async fn download(
        &self,
        repository_url: &str,
        requested_reference: Option<&str>,
        destination: &Path,
        maximum_bytes: u64,
    ) -> Result<GithubDownload, ApiError> {
        let locator = parse_repository_url(repository_url)?;
        let metadata_url = api_url(&["repos", &locator.owner, &locator.repository])?;
        let metadata_response = self.github_api_get(metadata_url).await?;
        require_success(&metadata_response, "github_repository_unavailable")?;
        let metadata_bytes = read_bounded(metadata_response, MAX_API_BYTES).await?;
        let metadata: RepositoryMetadata =
            serde_json::from_slice(&metadata_bytes).map_err(|_| {
                ApiError::upstream(
                    "github_invalid_response",
                    "GitHub returned invalid repository metadata.",
                )
            })?;
        if metadata.private
            || metadata
                .visibility
                .as_deref()
                .is_some_and(|v| v != "public")
        {
            return Err(ApiError::unprocessable(
                "github_public_only",
                "This version accepts public GitHub repositories only. Upload a ZIP instead.",
            ));
        }
        let (canonical_owner, canonical_repository) = split_full_name(&metadata.full_name)?;
        let reference = validate_reference(
            requested_reference
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(&metadata.default_branch),
        )?;

        let commit_url = api_url(&[
            "repos",
            &canonical_owner,
            &canonical_repository,
            "commits",
            &reference,
        ])?;
        let commit_response = self
            .client
            .get(commit_url)
            .header(ACCEPT, "application/vnd.github.sha")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .send()
            .await
            .map_err(|_| {
                ApiError::upstream(
                    "github_connection_failed",
                    "Could not resolve that GitHub reference. Check the connection and try again.",
                )
            })?;
        require_success(&commit_response, "github_reference_unavailable")?;
        let sha_body = read_bounded(commit_response, MAX_SHA_BYTES).await?;
        let commit_sha = std::str::from_utf8(&sha_body).map(str::trim).map_err(|_| {
            ApiError::upstream(
                "github_invalid_commit",
                "GitHub returned an invalid commit identity.",
            )
        })?;
        if commit_sha.len() != 40
            || !commit_sha
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(ApiError::upstream(
                "github_invalid_commit",
                "GitHub returned an invalid commit identity.",
            ));
        }

        let zipball_url = api_url(&[
            "repos",
            &canonical_owner,
            &canonical_repository,
            "zipball",
            commit_sha,
        ])?;
        let redirect_response = self.github_api_get(zipball_url).await?;
        if redirect_response.status() != StatusCode::FOUND {
            require_success(&redirect_response, "github_archive_unavailable")?;
            return Err(ApiError::upstream(
                "github_redirect_refused",
                "GitHub did not return the expected protected archive redirect.",
            ));
        }
        let location = redirect_response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                ApiError::upstream(
                    "github_redirect_refused",
                    "GitHub returned an archive redirect without a valid destination.",
                )
            })?;
        let download_url = validate_codeload_url(
            location,
            &canonical_owner,
            &canonical_repository,
            commit_sha,
        )?;
        let archive_response = self
            .client
            .get(download_url.clone())
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| {
                ApiError::upstream(
                    "github_download_failed",
                    "GitHub source download failed. Upload a ZIP instead or try again.",
                )
            })?;
        require_success(&archive_response, "github_archive_unavailable")?;
        let (archive_sha256, archive_bytes) =
            write_response_bounded(archive_response, destination, maximum_bytes).await?;

        Ok(GithubDownload {
            canonical_url: format!("https://github.com/{canonical_owner}/{canonical_repository}"),
            repository_id: metadata.id,
            requested_revision: reference,
            resolved_revision: commit_sha.to_owned(),
            retrieval_url: download_url.to_string(),
            archive_sha256,
            archive_bytes,
            suggested_name: canonical_repository,
        })
    }

    async fn github_api_get(&self, url: Url) -> Result<Response, ApiError> {
        self.client
            .get(url)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| {
                ApiError::upstream(
                    "github_connection_failed",
                    "Could not reach GitHub. Check the connection and try again.",
                )
            })
    }
}

fn parse_repository_url(value: &str) -> Result<RepositoryLocator, ApiError> {
    if value.len() > MAX_URL_BYTES || value.contains(['%', '\\', '\0']) {
        return Err(invalid_repository_url());
    }
    let authority = value
        .strip_prefix("https://")
        .and_then(|remaining| remaining.split('/').next());
    if authority.is_none_or(|authority| !authority.eq_ignore_ascii_case("github.com")) {
        return Err(invalid_repository_url());
    }
    let url = Url::parse(value).map_err(|_| invalid_repository_url())?;
    if url.scheme() != "https"
        || url.host_str().is_none_or(|host| host != "github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid_repository_url());
    }
    let segments = url
        .path_segments()
        .ok_or_else(invalid_repository_url)?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() != 2 {
        return Err(invalid_repository_url());
    }
    let owner = segments[0];
    let repository = segments[1].strip_suffix(".git").unwrap_or(segments[1]);
    if !valid_owner(owner) || !valid_repository(repository) {
        return Err(invalid_repository_url());
    }
    Ok(RepositoryLocator {
        owner: owner.to_owned(),
        repository: repository.to_owned(),
    })
}

fn valid_owner(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_repository(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn invalid_repository_url() -> ApiError {
    ApiError::bad_request(
        "invalid_github_url",
        "Enter a public repository root such as https://github.com/owner/repository.",
    )
}

fn split_full_name(value: &str) -> Result<(String, String), ApiError> {
    let (owner, repository) = value.split_once('/').ok_or_else(|| {
        ApiError::upstream(
            "github_invalid_response",
            "GitHub returned an invalid canonical repository name.",
        )
    })?;
    if value.matches('/').count() != 1 || !valid_owner(owner) || !valid_repository(repository) {
        return Err(ApiError::upstream(
            "github_invalid_response",
            "GitHub returned an invalid canonical repository name.",
        ));
    }
    Ok((owner.to_owned(), repository.to_owned()))
}

fn validate_reference(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_REF_BYTES
        || value.chars().any(char::is_control)
        || value.contains(['\\', '\0'])
    {
        return Err(ApiError::bad_request(
            "invalid_github_reference",
            "The branch, tag, or commit reference is invalid.",
        ));
    }
    Ok(value.to_owned())
}

fn api_url(segments: &[&str]) -> Result<Url, ApiError> {
    let mut url = Url::parse("https://api.github.com/")
        .map_err(|_| ApiError::internal("The GitHub API URL could not be initialized."))?;
    url.path_segments_mut()
        .map_err(|()| ApiError::internal("The GitHub API URL could not be built."))?
        .extend(segments.iter().copied());
    Ok(url)
}

fn validate_codeload_url(
    value: &str,
    owner: &str,
    repository: &str,
    commit_sha: &str,
) -> Result<Url, ApiError> {
    if value.len() > 4096 || value.contains(['\\', '\0']) {
        return Err(refused_redirect());
    }
    let url = Url::parse(value).map_err(|_| refused_redirect())?;
    if url.scheme() != "https"
        || url.host_str() != Some("codeload.github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(refused_redirect());
    }
    let segments = url
        .path_segments()
        .ok_or_else(refused_redirect)?
        .collect::<Vec<_>>();
    if segments.len() != 4
        || !segments[0].eq_ignore_ascii_case(owner)
        || !segments[1].eq_ignore_ascii_case(repository)
        || !matches!(segments[2], "legacy.zip" | "zip")
        || segments[3] != commit_sha
    {
        return Err(refused_redirect());
    }
    Ok(url)
}

fn refused_redirect() -> ApiError {
    ApiError::upstream(
        "github_redirect_refused",
        "GitHub redirected the archive to an unexpected destination, so the download was stopped.",
    )
}

fn require_success(response: &Response, code: &'static str) -> Result<(), ApiError> {
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let message = match status {
        StatusCode::NOT_FOUND => {
            "The public GitHub repository or reference was not found. Upload a ZIP instead."
                .to_owned()
        }
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => response
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|value| value.to_str().ok())
            .map_or_else(
                || "GitHub rate-limited this computer. Try later or upload a ZIP.".to_owned(),
                |reset| {
                    format!(
                        "GitHub rate-limited this computer until Unix time {reset}. Upload a ZIP or try later."
                    )
                },
            ),
        _ => format!("GitHub returned HTTP {status}. Upload a ZIP or try again."),
    };
    Err(ApiError::upstream(code, message))
}

async fn read_bounded(response: Response, maximum_bytes: u64) -> Result<Vec<u8>, ApiError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes)
    {
        return Err(ApiError::upstream(
            "github_response_too_large",
            "GitHub returned a response larger than the safe limit.",
        ));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            ApiError::upstream(
                "github_connection_failed",
                "The GitHub response ended unexpectedly.",
            )
        })?;
        let next = u64::try_from(bytes.len())
            .unwrap_or(u64::MAX)
            .saturating_add(chunk.len() as u64);
        if next > maximum_bytes {
            return Err(ApiError::upstream(
                "github_response_too_large",
                "GitHub returned a response larger than the safe limit.",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn write_response_bounded(
    response: Response,
    destination: &Path,
    maximum_bytes: u64,
) -> Result<(qbm_domain::Sha256Digest, u64), ApiError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes)
    {
        return Err(ApiError::too_large(format!(
            "The GitHub archive exceeds the {} MiB download limit.",
            maximum_bytes / 1024 / 1024
        )));
    }
    let mut output = tokio::fs::File::create(destination)
        .await
        .map_err(|_| ApiError::internal("The downloaded archive could not be staged."))?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            ApiError::upstream(
                "github_download_failed",
                "The GitHub archive download ended unexpectedly.",
            )
        })?;
        total = total.saturating_add(chunk.len() as u64);
        if total > maximum_bytes {
            return Err(ApiError::too_large(format!(
                "The GitHub archive exceeds the {} MiB download limit.",
                maximum_bytes / 1024 / 1024
            )));
        }
        hasher.update(&chunk);
        output
            .write_all(&chunk)
            .await
            .map_err(|_| ApiError::internal("The downloaded archive could not be staged."))?;
    }
    output
        .flush()
        .await
        .map_err(|_| ApiError::internal("The downloaded archive could not be committed."))?;
    let digest = qbm_domain::Sha256Digest::new(format!("{:x}", hasher.finalize()))
        .map_err(|_| ApiError::internal("The downloaded archive hash was invalid."))?;
    Ok((digest, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_repository_url_policy_rejects_spoofing_and_extra_paths() {
        for rejected in [
            "http://github.com/openai/openai-rust",
            "https://github.com.evil.test/openai/openai-rust",
            "https://user@github.com/openai/openai-rust",
            "https://github.com:443/openai/openai-rust",
            "https://github.com/openai/openai-rust/tree/main",
            "https://github.com/openai/%2e%2e",
            "https://github.com/openai/openai-rust?tab=readme",
            "git@github.com:openai/openai-rust.git",
        ] {
            assert!(
                parse_repository_url(rejected).is_err(),
                "accepted {rejected}"
            );
        }
    }

    #[test]
    fn public_repository_url_policy_accepts_the_three_root_forms() {
        for accepted in [
            "https://github.com/openai/openai-rust",
            "https://github.com/openai/openai-rust/",
            "https://github.com/openai/openai-rust.git",
        ] {
            let parsed = parse_repository_url(accepted).unwrap();
            assert_eq!(parsed.owner, "openai");
            assert_eq!(parsed.repository, "openai-rust");
        }
    }

    #[test]
    fn codeload_redirect_is_bound_to_repo_and_commit() {
        let sha = "a".repeat(40);
        let accepted = format!("https://codeload.github.com/openai/openai-rust/legacy.zip/{sha}");
        assert!(validate_codeload_url(&accepted, "openai", "openai-rust", &sha).is_ok());
        assert!(
            validate_codeload_url(
                &format!("https://codeload.github.com/evil/repo/legacy.zip/{sha}"),
                "openai",
                "openai-rust",
                &sha,
            )
            .is_err()
        );
    }
}
