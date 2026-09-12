//! Approval-gated biomedical profile, comparison, optimization and QUBO producers.

use std::{cmp::Ordering, path::Path};

use qbm_app::PlatformApp;
use qbm_domain::{Project, RunId, SourceAcquisition, SourceSnapshot, StageState};
use qbm_profiler::{
    Profiler, ProfilerLimits, ProjectionCandidate, ProjectionContext, SourceDocument,
};
use serde::{Deserialize, Serialize};

use crate::{
    benchmark::{
        BaselineReference, ClassicalOptimizationArtifact, ProfileComparisonArtifact,
        QuboValidationArtifact, analyze_profile, compare_profile_versions, validate_qubo_profile,
    },
    error::ApiError,
};

const MAXIMUM_PROFILE_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAXIMUM_DERIVED_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilityStatus {
    Complete,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilityArtifact<T> {
    pub schema_version: String,
    pub status: CapabilityStatus,
    pub value: Option<T>,
    pub skip_reason: Option<String>,
    pub limitations: Vec<String>,
}

impl<T> CapabilityArtifact<T> {
    fn complete(schema_version: &str, value: T, limitations: Vec<String>) -> Self {
        Self {
            schema_version: schema_version.to_owned(),
            status: CapabilityStatus::Complete,
            value: Some(value),
            skip_reason: None,
            limitations,
        }
    }

    fn skipped(schema_version: &str, reason: impl Into<String>, limitations: Vec<String>) -> Self {
        Self {
            schema_version: schema_version.to_owned(),
            status: CapabilityStatus::Skipped,
            value: None,
            skip_reason: Some(reason.into()),
            limitations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkippedDocument {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileStageArtifact {
    pub schema_version: String,
    pub status: CapabilityStatus,
    pub value: Option<ProjectionCandidate>,
    pub skip_reason: Option<String>,
    pub documents_considered: usize,
    pub documents_skipped: Vec<SkippedDocument>,
    pub semantic_projection_id: Option<String>,
    pub limitations: Vec<String>,
}

pub(crate) type ComparisonStageArtifact = CapabilityArtifact<ProfileComparisonArtifact>;
pub(crate) type OptimizationStageArtifact = CapabilityArtifact<ClassicalOptimizationArtifact>;
pub(crate) type QuboStageArtifact = CapabilityArtifact<QuboValidationArtifact>;

pub(crate) fn build_profile_stage(
    app: &PlatformApp,
    snapshot: &SourceSnapshot,
    project: &Project,
    acquisition: &SourceAcquisition,
    semantic_projection_id: Option<String>,
) -> ProfileStageArtifact {
    let limits = ProfilerLimits::default();
    let (documents, skipped) = source_documents(app, snapshot, limits.max_documents);
    let limitations = vec![
        "Only explicit qbm.profile documents or conservative structured biomedical keys can define optimizer semantics.".to_owned(),
        "Source-language symbol names are never treated as biomedical inputs or outcomes.".to_owned(),
        "Any default cost or outcome weight is recorded as a visible assumption and requires exact-result approval.".to_owned(),
    ];
    if documents.is_empty() {
        return ProfileStageArtifact {
            schema_version: "qbm.profile-stage/v1".to_owned(),
            status: CapabilityStatus::Skipped,
            value: None,
            skip_reason: Some(
                "No bounded JSON or YAML document could be supplied to the biomedical profiler. Add an explicit qbm.profile.json or qbm.profile.yaml file."
                    .to_owned(),
            ),
            documents_considered: 0,
            documents_skipped: skipped,
            semantic_projection_id,
            limitations,
        };
    }
    let context = ProjectionContext {
        profile_id: Some(stable_profile_id(&project.display_name)),
        title: Some(project.display_name.clone()),
        biomedical_area: Some(inferred_biomedical_area(
            &project.display_name,
            &acquisition.source_locator,
        )),
        population: Some(project.display_name.clone()),
        source_revision: acquisition.resolved_revision.clone(),
        generated_by: Some("qbenchmed-platform/builtin-biomedical-profiler-v1".to_owned()),
    };
    let projected = Profiler::with_limits(limits).project_with_context(&documents, &context);
    match projected {
        Ok(candidate) => ProfileStageArtifact {
            schema_version: "qbm.profile-stage/v1".to_owned(),
            status: CapabilityStatus::Complete,
            value: Some(candidate),
            skip_reason: None,
            documents_considered: documents.len(),
            documents_skipped: skipped,
            semantic_projection_id,
            limitations,
        },
        Err(error) => ProfileStageArtifact {
            schema_version: "qbm.profile-stage/v1".to_owned(),
            status: CapabilityStatus::Skipped,
            value: None,
            skip_reason: Some(format!(
                "{error}. Add or export an explicit qbm.profile.json/YAML to enable optimization without heuristic interpretation."
            )),
            documents_considered: documents.len(),
            documents_skipped: skipped,
            semantic_projection_id,
            limitations,
        },
    }
}

pub(crate) fn build_comparison_stage(
    app: &PlatformApp,
    run_id: RunId,
    profile_artifact_hash: &str,
) -> Result<ComparisonStageArtifact, ApiError> {
    let current: ProfileStageArtifact = load_artifact(app, profile_artifact_hash)?;
    let Some(candidate) = current.value.as_ref() else {
        return Ok(CapabilityArtifact::skipped(
            "qbm.profile-comparison-stage/v1",
            "No approved benchmark profile exists in this run, so semantic version comparison cannot be calculated.",
            vec!["Source revisions can be compared only after both revisions produce approved profiles with the same profile_id.".to_owned()],
        ));
    };
    let baseline = find_baseline(app, run_id, candidate)?;
    let baseline_tuple = baseline
        .as_ref()
        .map(|(reference, candidate)| (reference, &candidate.profile));
    match compare_profile_versions(&candidate.profile, profile_artifact_hash, baseline_tuple) {
        Ok(value) => Ok(CapabilityArtifact::complete(
            "qbm.profile-comparison-stage/v1",
            value,
            Vec::new(),
        )),
        Err(reason) => Ok(CapabilityArtifact::skipped(
            "qbm.profile-comparison-stage/v1",
            reason,
            vec!["A failed comparison does not alter the approved current profile.".to_owned()],
        )),
    }
}

pub(crate) fn build_optimization_stage(
    app: &PlatformApp,
    profile_artifact_hash: &str,
) -> Result<OptimizationStageArtifact, ApiError> {
    let current: ProfileStageArtifact = load_artifact(app, profile_artifact_hash)?;
    let Some(candidate) = current.value.as_ref() else {
        return Ok(CapabilityArtifact::skipped(
            "qbm.classical-optimization-stage/v1",
            "Classical optimization requires an approved qbm.profile containing inputs, outcomes and relationships.",
            vec!["No solver was executed.".to_owned()],
        ));
    };
    Ok(match analyze_profile(&candidate.profile, profile_artifact_hash) {
        Ok(value) => CapabilityArtifact::complete(
            "qbm.classical-optimization-stage/v1",
            value,
            Vec::new(),
        ),
        Err(reason) => CapabilityArtifact::skipped(
            "qbm.classical-optimization-stage/v1",
            reason,
            vec!["The approved profile remains available for correction or external analysis; no partial solver result is presented as complete.".to_owned()],
        ),
    })
}

pub(crate) fn build_qubo_stage(
    app: &PlatformApp,
    profile_artifact_hash: &str,
) -> Result<QuboStageArtifact, ApiError> {
    let current: ProfileStageArtifact = load_artifact(app, profile_artifact_hash)?;
    let Some(candidate) = current.value.as_ref() else {
        return Ok(CapabilityArtifact::skipped(
            "qbm.qubo-ising-stage/v1",
            "QUBO/Ising construction requires an approved qbm.profile.",
            vec![
                "No logical quantum model was constructed and no provider request was attempted."
                    .to_owned(),
            ],
        ));
    };
    Ok(match validate_qubo_profile(&candidate.profile, profile_artifact_hash) {
        Ok(value) => CapabilityArtifact::complete(
            "qbm.qubo-ising-stage/v1",
            value,
            Vec::new(),
        ),
        Err(reason) => CapabilityArtifact::skipped(
            "qbm.qubo-ising-stage/v1",
            reason,
            vec![
                "Unsupported constraints are reported rather than silently dropped from the QUBO.".to_owned(),
                "No provider request was attempted.".to_owned(),
            ],
        ),
    })
}

pub(crate) fn load_artifact<T: serde::de::DeserializeOwned>(
    app: &PlatformApp,
    hash: &str,
) -> Result<T, ApiError> {
    Ok(app.load_json_artifact_bounded(hash, MAXIMUM_DERIVED_ARTIFACT_BYTES)?)
}

fn source_documents(
    app: &PlatformApp,
    snapshot: &SourceSnapshot,
    maximum_documents: usize,
) -> (Vec<SourceDocument>, Vec<SkippedDocument>) {
    let mut artifacts = snapshot.artifacts.iter().collect::<Vec<_>>();
    artifacts.sort_by(|left, right| {
        profile_path_priority(&left.relative_path)
            .cmp(&profile_path_priority(&right.relative_path))
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    let mut documents = Vec::new();
    let mut skipped = Vec::new();
    for artifact in artifacts {
        let extension = Path::new(&artifact.relative_path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "json" | "yaml" | "yml") {
            continue;
        }
        if documents.len() >= maximum_documents {
            skipped.push(SkippedDocument {
                path: artifact.relative_path.clone(),
                reason: format!(
                    "Structured profiler document limit of {maximum_documents} was reached."
                ),
            });
            continue;
        }
        let bytes = match app
            .read_artifact_bounded(artifact.sha256.as_str(), MAXIMUM_PROFILE_SOURCE_BYTES)
        {
            Ok(bytes) => bytes,
            Err(error) => {
                skipped.push(SkippedDocument {
                    path: artifact.relative_path.clone(),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let parsed = if extension == "json" {
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())
        } else {
            parse_yaml_documents(&bytes)
        };
        match parsed {
            Ok(value) => documents.push(SourceDocument::new(
                artifact.relative_path.clone(),
                artifact.sha256.to_string(),
                value,
            )),
            Err(reason) => skipped.push(SkippedDocument {
                path: artifact.relative_path.clone(),
                reason,
            }),
        }
    }
    skipped.sort_by(|left, right| left.path.cmp(&right.path));
    (documents, skipped)
}

fn parse_yaml_documents(bytes: &[u8]) -> Result<serde_json::Value, String> {
    let mut documents = Vec::new();
    for document in serde_yaml::Deserializer::from_slice(bytes) {
        documents
            .push(serde_json::Value::deserialize(document).map_err(|error| error.to_string())?);
    }
    match documents.len() {
        0 => Ok(serde_json::Value::Null),
        1 => Ok(documents.pop().expect("one document exists")),
        _ => Ok(serde_json::Value::Array(documents)),
    }
}

fn profile_path_priority(path: &str) -> u8 {
    let lower = path.to_ascii_lowercase();
    if lower.contains("qbm.profile") || lower.contains("qbm-profile") {
        return 0;
    }
    if [
        "parameter",
        "input",
        "biomarker",
        "outcome",
        "action",
        "option",
        "decision",
        "rule",
        "context",
        "dependency",
        "registry",
        "oncology",
        "nsclc",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
    {
        return 1;
    }
    2
}

fn stable_profile_id(display_name: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in display_name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            separator = false;
        } else if !separator && !slug.is_empty() {
            slug.push('-');
            separator = true;
        }
        if slug.len() >= 200 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("project");
    }
    format!("biomedical/{slug}")
}

fn inferred_biomedical_area(display_name: &str, source_locator: &str) -> String {
    let haystack = format!("{display_name} {source_locator}").to_ascii_lowercase();
    if ["oncology", "cancer", "tumor", "tumour", "nsclc"]
        .iter()
        .any(|value| haystack.contains(value))
    {
        "oncology".to_owned()
    } else if haystack.contains("radiolog") {
        "radiology".to_owned()
    } else if haystack.contains("molecul") || haystack.contains("drug") {
        "molecular_biology".to_owned()
    } else {
        "biomedical".to_owned()
    }
}

fn find_baseline(
    app: &PlatformApp,
    current_run_id: RunId,
    current: &ProjectionCandidate,
) -> Result<Option<(BaselineReference, ProjectionCandidate)>, ApiError> {
    let mut runs = app.runs(None)?;
    runs.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| run_id_order(left.id, right.id))
    });
    for run in runs {
        if run.id == current_run_id {
            continue;
        }
        let Some(output) = app.stage_output(run.id, "biomedical-profile")? else {
            continue;
        };
        if output.state != StageState::Approved {
            continue;
        }
        let prior: ProfileStageArtifact = match load_artifact(app, output.output_hash.as_str()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(candidate) = prior.value else {
            continue;
        };
        if candidate.profile.profile_id != current.profile.profile_id {
            continue;
        }
        let revision = app
            .run_acquisition(run.id)
            .ok()
            .and_then(|acquisition| acquisition.resolved_revision);
        return Ok(Some((
            BaselineReference {
                run_id: run.id.to_string(),
                profile_artifact_hash: output.output_hash.to_string(),
                source_revision: revision,
            },
            candidate,
        )));
    }
    Ok(None)
}

fn run_id_order(left: RunId, right: RunId) -> Ordering {
    left.to_string().cmp(&right.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_profile_identity_is_version_independent_and_portable() {
        assert_eq!(
            stable_profile_id("NSCLC Project"),
            "biomedical/nsclc-project"
        );
        assert_eq!(stable_profile_id("---"), "biomedical/project");
    }

    #[test]
    fn biomedical_area_is_conservative() {
        assert_eq!(inferred_biomedical_area("NSCLC", "upload"), "oncology");
        assert_eq!(inferred_biomedical_area("Assay", "folder"), "biomedical");
    }
}
