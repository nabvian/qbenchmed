//! Strict, versioned input schema for biomedical panel optimization.

use serde::{Deserialize, Serialize};

/// Original binary-incidence benchmark-profile schema identifier.
///
/// A `v1` document carries only untyped `(input_id, outcome_id)` pairs. It is
/// still accepted, and loads as a profile in which every relationship is
/// `supporting` and occupies its own singleton arm — exactly the disjunctive
/// "any one selected input covers the outcome" semantics `v1` meant.
pub const BENCHMARK_PROFILE_SCHEMA_V1: &str = "qbm.benchmark-profile/v1";

/// Current benchmark-profile schema identifier.
///
/// `v2` adds relationship kinds and path groups, so an outcome can require a
/// *combination* of inputs rather than any one of them. That is what lets a
/// profile represent a real rule set, where a trigger fires only when several
/// inputs are present together — and it is why coverage under `v2` is not
/// submodular and greedy carries no approximation guarantee.
pub const BENCHMARK_PROFILE_SCHEMA_V2: &str = "qbm.benchmark-profile/v2";

/// Schema identifier emitted by this build.
pub const BENCHMARK_PROFILE_SCHEMA_VERSION: &str = BENCHMARK_PROFILE_SCHEMA_V2;

/// Every profile schema version this build accepts, newest first.
pub const SUPPORTED_PROFILE_SCHEMA_VERSIONS: [&str; 2] =
    [BENCHMARK_PROFILE_SCHEMA_V2, BENCHMARK_PROFILE_SCHEMA_V1];

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

/// How one relationship participates in deciding whether an outcome is covered.
///
/// The kinds mirror the reference implementation's typed edge model. Reducing
/// every biomedical relationship to a plain binary incidence loses the
/// distinction between "this input helps" and "this input is mandatory", which
/// is the distinction that decides whether a rule actually fires.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    /// The outcome cannot be covered unless this input is selected.
    ///
    /// An unpathed `required` edge is both a global precondition and, taken
    /// together with the outcome's other unpathed `required` edges, one arm —
    /// so an outcome whose only edge is `required` is covered once that input
    /// is selected.
    Required,
    /// The input contributes to the outcome. This is the plain binary edge, and
    /// the kind every `v1` relationship loads as.
    #[default]
    Supporting,
    /// The input contributes, and is recorded as carrying less evidential
    /// weight than a full supporting edge.
    Optional,
    /// Selecting this input *blocks* the outcome.
    Exclusionary,
    /// The input contributes only when its `context_input_id` is also selected.
    Contextual,
}

impl RelationshipKind {
    /// Whether this kind grants coverage credit when its input is selected.
    #[must_use]
    pub const fn contributes(self) -> bool {
        !matches!(self, Self::Exclusionary)
    }

    /// Stable lowercase name used in diagnostics and exports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Supporting => "supporting",
            Self::Optional => "optional",
            Self::Exclusionary => "exclusionary",
            Self::Contextual => "contextual",
        }
    }
}

/// One typed relationship between a selectable input and a coverable outcome.
///
/// Relationships sharing an outcome and a non-empty `path` form a *conjunction*:
/// that arm of the outcome's rule is satisfied only when all of its members are
/// satisfied together. An outcome is covered when any one of its arms is
/// satisfied. An empty `path` means the edge is its own singleton arm, which is
/// why a document with no paths behaves exactly like binary incidence.
///
/// Ordering is `(input_id, outcome_id, path, kind, context_input_id)`, so a
/// `v1` document — where `path` is empty everywhere — sorts identically under
/// both schema versions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncidenceRelationship {
    /// Referenced input identity.
    pub input_id: String,
    /// Referenced outcome identity.
    pub outcome_id: String,
    /// Rule arm this edge belongs to. Empty means "its own singleton arm".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// How this edge participates in coverage. Absent means `supporting`.
    #[serde(default, skip_serializing_if = "is_default_kind")]
    pub kind: RelationshipKind,
    /// For `contextual` edges only: the input that must also be selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_input_id: Option<String>,
}

// serde's `skip_serializing_if` hands the field by reference, so this cannot
// take the value even though the value is one byte.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_default_kind(kind: &RelationshipKind) -> bool {
    *kind == RelationshipKind::Supporting
}

impl IncidenceRelationship {
    /// Construct a plain binary `supporting` incidence, the `v1` shape.
    #[must_use]
    pub fn supporting(input_id: impl Into<String>, outcome_id: impl Into<String>) -> Self {
        Self {
            input_id: input_id.into(),
            outcome_id: outcome_id.into(),
            path: String::new(),
            kind: RelationshipKind::Supporting,
            context_input_id: None,
        }
    }

    /// Whether this relationship uses any feature absent from schema `v1`.
    #[must_use]
    pub fn uses_typed_features(&self) -> bool {
        !self.path.is_empty()
            || self.kind != RelationshipKind::Supporting
            || self.context_input_id.is_some()
    }
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
    /// Relationships sorted by `(input_id, outcome_id, path, kind, context_input_id)`.
    pub relationships: Vec<IncidenceRelationship>,
    /// Sorted outcomes that are covered by every selection, including the empty
    /// one.
    ///
    /// A real rule set can contain a fallback that fires when nothing else
    /// matches. Such an outcome is not an error, but it inflates every coverage
    /// ratio, so it is declared here and reported separately rather than being
    /// silently folded into the score.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unconditional_outcomes: Vec<String>,
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

    /// Whether this profile uses any feature absent from schema `v1`.
    ///
    /// A `v1` document that uses one is rejected rather than silently upgraded:
    /// the version a document declares has to mean what it says.
    #[must_use]
    pub fn uses_typed_features(&self) -> bool {
        !self.unconditional_outcomes.is_empty()
            || self
                .relationships
                .iter()
                .any(IncidenceRelationship::uses_typed_features)
    }

    /// Whether coverage over this profile reduces to plain binary incidence.
    ///
    /// When true, coverage is monotone submodular and greedy carries the
    /// classical `1 - 1/e` guarantee. When false, adding an input can *remove*
    /// coverage and that guarantee does not hold.
    #[must_use]
    pub fn is_purely_disjunctive(&self) -> bool {
        !self.uses_typed_features()
    }
}
