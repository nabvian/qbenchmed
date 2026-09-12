//! Strict, versioned input schema for biomedical panel optimization.

use serde::{Deserialize, Serialize};

/// Accepted benchmark-profile schema identifier.
pub const BENCHMARK_PROFILE_SCHEMA_VERSION: &str = "qbm.benchmark-profile/v1";

/// Explicit biomedical scope of a projected profile.
///
/// The free-text fields describe the reviewed study/model boundary. They do not
/// permit the engine to optimize unrelated general software concerns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiomedicalScope {
    /// Biomedical area, for example `oncology` or `molecular_biology`.
    pub area: String,
    /// Population, specimen, cohort, or experimental boundary.
    pub population: String,
    /// Meaning of an input in this profile, such as biomarker or assay.
    pub input_semantics: String,
    /// Meaning of an outcome, such as covered clinical decision or phenotype.
    pub outcome_semantics: String,
}

/// One selectable biomedical input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkInput {
    /// Stable identity used in relationships and version comparisons.
    pub id: String,
    /// Human-readable input name.
    pub label: String,
    /// Strictly positive acquisition/measurement cost in declared profile units.
    pub cost: f64,
    /// Deterministically sorted biomedical category tags.
    pub tags: Vec<String>,
}

/// One biomedical outcome that an input panel may cover.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkOutcome {
    /// Stable identity used in relationships and version comparisons.
    pub id: String,
    /// Human-readable outcome name.
    pub label: String,
    /// Strictly positive importance used for weighted coverage.
    pub weight: f64,
    /// Deterministically sorted biomedical category tags.
    pub tags: Vec<String>,
}

/// Binary incidence between one selectable input and one covered outcome.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncidenceRelationship {
    /// Referenced input identity.
    pub input_id: String,
    /// Referenced outcome identity.
    pub outcome_id: String,
}

/// Feasibility constraints declared by the reviewed biomedical profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkConstraints {
    /// Minimum number of selected inputs.
    pub min_selected: usize,
    /// Optional maximum number of selected inputs.
    pub max_selected: Option<usize>,
    /// Optional maximum total cost; values must use the input cost unit.
    pub max_total_cost: Option<f64>,
    /// Sorted stable identities that every selection must contain.
    pub required_inputs: Vec<String>,
    /// Sorted stable identities that every selection must omit.
    pub excluded_inputs: Vec<String>,
    /// Sorted outcomes that every feasible selection must cover.
    pub required_outcomes: Vec<String>,
}

/// Supported primary optimization objective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkObjective {
    /// Maximize the total weight of outcomes covered at least once.
    MaximizeWeightedCoverage,
}

/// Auditable origin of the projected profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileProvenance {
    /// Adapter/exporter identity and version.
    pub generated_by: String,
    /// Immutable source revision when one exists.
    pub source_revision: Option<String>,
    /// Sorted immutable artifact or evidence identities supporting projection.
    pub source_artifact_ids: Vec<String>,
    /// Human-readable projection method or approval reference.
    pub projection_method: String,
}

/// Complete, deterministic input to the Q-BenchMed optimization kernel.
///
/// Collections must be sorted as documented; validation rejects duplicates,
/// dangling references, unknown versions, and non-finite/non-positive numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkProfile {
    /// Must equal [`BENCHMARK_PROFILE_SCHEMA_VERSION`].
    pub schema_version: String,
    /// Stable profile identity across versions.
    pub profile_id: String,
    /// Human-readable profile title.
    pub title: String,
    /// Explicit biomedical boundary.
    pub biomedical_scope: BiomedicalScope,
    /// Inputs sorted by stable identity.
    pub inputs: Vec<BenchmarkInput>,
    /// Outcomes sorted by stable identity.
    pub outcomes: Vec<BenchmarkOutcome>,
    /// Relationships sorted by `(input_id, outcome_id)`.
    pub relationships: Vec<IncidenceRelationship>,
    /// Feasibility rules.
    pub constraints: BenchmarkConstraints,
    /// Primary optimization objective.
    pub objective: BenchmarkObjective,
    /// Evidence needed to reproduce the projection.
    pub provenance: ProfileProvenance,
}

impl BenchmarkProfile {
    /// Validate schema version, ordering, references, numeric bounds, and constraints.
    pub fn validate(&self) -> Result<(), crate::BenchmarkError> {
        crate::validation::validate_profile(self)
    }
}
