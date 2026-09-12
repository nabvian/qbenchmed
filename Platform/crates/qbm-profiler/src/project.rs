use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

use qbm_benchmark::{
    BENCHMARK_PROFILE_SCHEMA_VERSION, BenchmarkConstraints, BenchmarkInput, BenchmarkObjective,
    BenchmarkOutcome, BenchmarkProfile, BiomedicalScope, IncidenceRelationship, ProfileProvenance,
};
use serde_json::{Map, Value};

use crate::{
    DiagnosticSeverity, PROJECTION_CANDIDATE_SCHEMA_VERSION, ProfilerError, ProfilerLimits,
    ProjectionAssumption, ProjectionCandidate, ProjectionContext, ProjectionDiagnostic,
    ProjectionEvidence, ProjectionMethod, ProjectionStatus, SourceDocument,
};

const INPUT_ID_KEYS: &[&str] = &["biomarker_id", "feature_id", "input_id", "parameter_id"];
const OUTCOME_ID_KEYS: &[&str] = &["action_id", "option_id", "outcome", "outcome_id"];
const CONDITION_INPUT_KEYS: &[&str] = &["field", "input", "input_id", "parameter_id"];
const EMISSION_KEYS: &[&str] = &["action", "action_id", "option_id", "outcome", "outcome_id"];
const CONTEXT_KEYS: &[&str] = &[
    "area",
    "biomedical_area",
    "cancer_type",
    "clinical_domain",
    "disease",
    "domain",
    "indication",
    "population",
    "therapeutic_area",
];
const POPULATION_KEYS: &[&str] = &[
    "cancer_type",
    "cohort",
    "disease",
    "indication",
    "population",
];
const AREA_KEYS: &[&str] = &[
    "area",
    "biomedical_area",
    "clinical_domain",
    "domain",
    "therapeutic_area",
];

/// Stateless projector for bounded, pre-parsed biomedical documents.
#[derive(Debug, Clone, Copy)]
pub struct Profiler {
    limits: ProfilerLimits,
}

impl Default for Profiler {
    fn default() -> Self {
        Self::new()
    }
}

impl Profiler {
    /// Construct a profiler with conservative built-in limits.
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(ProfilerLimits::default())
    }

    /// Construct a profiler with host-controlled immutable limits.
    #[must_use]
    pub const fn with_limits(limits: ProfilerLimits) -> Self {
        Self { limits }
    }

    /// Return the configured limits.
    #[must_use]
    pub const fn limits(&self) -> ProfilerLimits {
        self.limits
    }

    /// Project one valid, reviewable benchmark-profile candidate.
    ///
    /// Explicit canonical profiles take precedence. Otherwise, projection uses
    /// only the documented structured keys and requires a structured biomedical
    /// context marker plus at least one rule-derived relationship.
    pub fn project(
        &self,
        documents: &[SourceDocument],
    ) -> Result<ProjectionCandidate, ProfilerError> {
        self.project_with_context(documents, &ProjectionContext::default())
    }

    /// Project with caller-owned stable metadata for cross-version comparison.
    pub fn project_with_context(
        &self,
        documents: &[SourceDocument],
        context: &ProjectionContext,
    ) -> Result<ProjectionCandidate, ProfilerError> {
        validate_projection_context(context)?;
        let ordered = self.validate_and_order_documents(documents)?;
        let mut explicit = Vec::new();
        for document in &ordered {
            if let Some(candidate) = explicit_payload(document)? {
                explicit.push(candidate);
            }
        }
        if explicit.len() > 1 {
            return Err(ProfilerError::AmbiguousExplicitProfiles(
                explicit
                    .iter()
                    .map(|item| item.document.path.clone())
                    .collect(),
            ));
        }
        if let Some(item) = explicit.into_iter().next() {
            return self.project_explicit(item);
        }
        self.project_heuristic(&ordered, context)
    }

    fn validate_and_order_documents<'a>(
        &self,
        documents: &'a [SourceDocument],
    ) -> Result<Vec<&'a SourceDocument>, ProfilerError> {
        if documents.is_empty() {
            return Err(ProfilerError::EmptyInput);
        }
        if documents.len() > self.limits.max_documents {
            return Err(resource("source documents", self.limits.max_documents));
        }

        let mut budget = ValueBudget::default();
        let mut paths = BTreeSet::new();
        for document in documents {
            validate_source_metadata(document)?;
            if !paths.insert(document.path.as_str()) {
                return Err(document_error(
                    document,
                    "a path may appear only once in a projection request",
                ));
            }
            measure_value(&document.value, 1, &mut budget, self.limits)?;
        }

        let mut ordered: Vec<_> = documents.iter().collect();
        ordered.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.artifact_id.cmp(&right.artifact_id))
        });
        Ok(ordered)
    }

    fn project_explicit(
        &self,
        explicit: ExplicitDocument<'_>,
    ) -> Result<ProjectionCandidate, ProfilerError> {
        let mut profile: BenchmarkProfile = serde_json::from_value(explicit.payload.clone())
            .map_err(|error| ProfilerError::InvalidExplicitProfile {
                path: explicit.document.path.clone(),
                message: error.to_string(),
            })?;
        let added_profile_artifact = !profile
            .provenance
            .source_artifact_ids
            .contains(&explicit.document.artifact_id);
        if added_profile_artifact {
            profile
                .provenance
                .source_artifact_ids
                .push(explicit.document.artifact_id.clone());
            profile.provenance.source_artifact_ids.sort();
            profile.provenance.source_artifact_ids.dedup();
        }
        profile.validate().map_err(ProfilerError::InvalidProfile)?;

        let root_point = SourcePoint::new(explicit.document, &explicit.pointer);
        let mut builder = CandidateBuilder::default();
        builder.add_evidence("/profile", std::slice::from_ref(&root_point));
        collect_explicit_fields(
            explicit.payload,
            "",
            &explicit.pointer,
            explicit.document,
            &mut builder,
        );
        builder.add_diagnostic(
            "approval_required",
            DiagnosticSeverity::Info,
            "A valid explicit profile was extracted verbatim; human approval is still required",
            "/profile",
            std::slice::from_ref(&root_point),
        );
        if added_profile_artifact {
            let target = format!(
                "/provenance/source_artifact_ids/{}",
                escape_pointer_segment(&explicit.document.artifact_id)
            );
            builder.add_default(
                &target,
                "explicit_profile_artifact_provenance",
                &explicit.document.artifact_id,
                "The explicit profile artifact was added as projection provenance",
                std::slice::from_ref(&root_point),
            );
            builder.add_diagnostic(
                "profile_artifact_added_to_provenance",
                DiagnosticSeverity::Info,
                "The explicit profile artifact was added to profile.provenance.source_artifact_ids",
                &target,
                std::slice::from_ref(&root_point),
            );
        }
        let (extracted_fields, defaulted_fields, evidence, diagnostics, assumptions) =
            builder.finish(self.limits)?;
        let candidate = ProjectionCandidate {
            schema_version: PROJECTION_CANDIDATE_SCHEMA_VERSION.to_owned(),
            method: ProjectionMethod::ExplicitProfile,
            status: ProjectionStatus::RequiresApproval,
            confidence_bps: if added_profile_artifact {
                9_900
            } else {
                10_000
            },
            profile,
            extracted_fields,
            defaulted_fields,
            evidence,
            diagnostics,
            assumptions,
        };
        candidate.validate()?;
        Ok(candidate)
    }

    fn project_heuristic(
        &self,
        documents: &[&SourceDocument],
        context: &ProjectionContext,
    ) -> Result<ProjectionCandidate, ProfilerError> {
        let mut scan = ScanState::default();
        for document in documents {
            scan_document(document, &mut scan, self.limits)?;
        }
        if scan.contexts.is_empty() {
            return Err(ProfilerError::NoProjection(
                "no recognized biomedical or oncology value was found under a structured context key"
                    .to_owned(),
            ));
        }
        if scan.relationships.is_empty() {
            return Err(ProfilerError::NoProjection(
                "no rule linked a structured condition input to an emitted outcome or action"
                    .to_owned(),
            ));
        }
        if scan.inputs.is_empty() || scan.outcomes.is_empty() {
            return Err(ProfilerError::NoProjection(
                "both structured biomedical inputs and outcomes are required".to_owned(),
            ));
        }
        if scan.inputs.len() > self.limits.max_inputs {
            return Err(resource("projected inputs", self.limits.max_inputs));
        }
        if scan.outcomes.len() > self.limits.max_outcomes {
            return Err(resource("projected outcomes", self.limits.max_outcomes));
        }
        if scan.relationships.len() > self.limits.max_relationships {
            return Err(resource(
                "projected relationships",
                self.limits.max_relationships,
            ));
        }

        build_heuristic_candidate(scan, documents, context, self.limits)
    }
}

#[derive(Debug)]
struct ExplicitDocument<'a> {
    document: &'a SourceDocument,
    payload: &'a Value,
    pointer: String,
}

fn explicit_payload(
    document: &SourceDocument,
) -> Result<Option<ExplicitDocument<'_>>, ProfilerError> {
    let Value::Object(object) = &document.value else {
        if is_canonical_profile_path(&document.path) {
            return Err(ProfilerError::InvalidExplicitProfile {
                path: document.path.clone(),
                message: "canonical profile document must contain an object".to_owned(),
            });
        }
        return Ok(None);
    };

    let dotted = object.get("qbm.profile");
    let dashed = object.get("qbm-profile");
    if dotted.is_some() && dashed.is_some() {
        return Err(ProfilerError::InvalidExplicitProfile {
            path: document.path.clone(),
            message: "document cannot contain both qbm.profile and qbm-profile envelopes"
                .to_owned(),
        });
    }
    if let Some((payload, key)) = dotted
        .map(|value| (value, "qbm.profile"))
        .or_else(|| dashed.map(|value| (value, "qbm-profile")))
    {
        if !payload.is_object() {
            return Err(ProfilerError::InvalidExplicitProfile {
                path: document.path.clone(),
                message: format!("{key} envelope must contain an object"),
            });
        }
        return Ok(Some(ExplicitDocument {
            document,
            payload,
            pointer: format!("/{}", escape_pointer_segment(key)),
        }));
    }

    if object.get("schema_version").and_then(Value::as_str)
        == Some(BENCHMARK_PROFILE_SCHEMA_VERSION)
    {
        return Ok(Some(ExplicitDocument {
            document,
            payload: &document.value,
            pointer: String::new(),
        }));
    }

    let marker = object
        .get("kind")
        .or_else(|| object.get("schema"))
        .and_then(Value::as_str);
    if marker.is_some_and(is_profile_marker) {
        let payload =
            object
                .get("profile")
                .ok_or_else(|| ProfilerError::InvalidExplicitProfile {
                    path: document.path.clone(),
                    message: "qbm.profile/qbm-profile envelope is missing object field profile"
                        .to_owned(),
                })?;
        if !payload.is_object() {
            return Err(ProfilerError::InvalidExplicitProfile {
                path: document.path.clone(),
                message: "profile envelope field must contain an object".to_owned(),
            });
        }
        return Ok(Some(ExplicitDocument {
            document,
            payload,
            pointer: "/profile".to_owned(),
        }));
    }

    if is_canonical_profile_path(&document.path) {
        let (payload, pointer) = object
            .get("profile")
            .map_or((&document.value, String::new()), |value| {
                (value, "/profile".to_owned())
            });
        if !payload.is_object() {
            return Err(ProfilerError::InvalidExplicitProfile {
                path: document.path.clone(),
                message: "canonical profile payload must contain an object".to_owned(),
            });
        }
        return Ok(Some(ExplicitDocument {
            document,
            payload,
            pointer,
        }));
    }
    Ok(None)
}

fn is_profile_marker(value: &str) -> bool {
    matches!(
        value,
        "qbm.profile" | "qbm-profile" | "qbm.profile/v1" | "qbm-profile/v1"
    )
}

fn is_canonical_profile_path(path: &str) -> bool {
    matches!(
        path.rsplit('/').next(),
        Some(
            "qbm.profile"
                | "qbm.profile.json"
                | "qbm.profile.yaml"
                | "qbm.profile.yml"
                | "qbm-profile"
                | "qbm-profile.json"
                | "qbm-profile.yaml"
                | "qbm-profile.yml"
        )
    )
}

fn collect_explicit_fields(
    value: &Value,
    target_pointer: &str,
    source_pointer: &str,
    document: &SourceDocument,
    builder: &mut CandidateBuilder,
) {
    match value {
        Value::Array(items) if !items.is_empty() => {
            for (index, item) in items.iter().enumerate() {
                let segment = index.to_string();
                collect_explicit_fields(
                    item,
                    &join_pointer(target_pointer, &segment),
                    &join_pointer(source_pointer, &segment),
                    document,
                    builder,
                );
            }
        }
        Value::Object(object) if !object.is_empty() => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();
            for key in keys {
                collect_explicit_fields(
                    &object[key],
                    &join_pointer(target_pointer, key),
                    &join_pointer(source_pointer, key),
                    document,
                    builder,
                );
            }
        }
        _ => {
            let target = if target_pointer.is_empty() {
                "/profile".to_owned()
            } else {
                target_pointer.to_owned()
            };
            builder.extracted_fields.insert(target.clone());
            builder.add_evidence(&target, &[SourcePoint::new(document, source_pointer)]);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SourcePoint {
    artifact_id: String,
    path: String,
    pointer: String,
}

impl SourcePoint {
    fn new(document: &SourceDocument, pointer: &str) -> Self {
        Self {
            artifact_id: document.artifact_id.clone(),
            path: document.path.clone(),
            pointer: pointer.to_owned(),
        }
    }

    fn to_evidence(&self, target_field: &str) -> ProjectionEvidence {
        ProjectionEvidence {
            artifact_id: self.artifact_id.clone(),
            path: self.path.clone(),
            pointer: self.pointer.clone(),
            target_field: target_field.to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
struct TextObservation {
    value: String,
    point: SourcePoint,
}

#[derive(Debug, Clone)]
struct NumberObservation {
    value: f64,
    point: SourcePoint,
}

#[derive(Debug, Clone)]
struct EntityObservation {
    id_point: SourcePoint,
    label: Option<TextObservation>,
    number: Option<NumberObservation>,
    tags: Vec<TextObservation>,
}

#[derive(Debug, Clone)]
struct ContextObservation {
    key: String,
    value: String,
    point: SourcePoint,
}

#[derive(Debug, Clone)]
struct PendingDiagnostic {
    code: String,
    severity: DiagnosticSeverity,
    message: String,
    target_field: String,
    points: Vec<SourcePoint>,
}

#[derive(Debug, Default)]
struct ScanState {
    inputs: BTreeMap<String, Vec<EntityObservation>>,
    outcomes: BTreeMap<String, Vec<EntityObservation>>,
    relationships: BTreeMap<(String, String), BTreeSet<SourcePoint>>,
    contexts: Vec<ContextObservation>,
    profile_ids: Vec<TextObservation>,
    titles: Vec<TextObservation>,
    diagnostics: Vec<PendingDiagnostic>,
    used_artifact_ids: BTreeSet<String>,
    rules_seen: usize,
}

impl ScanState {
    fn diagnostic(
        &mut self,
        code: impl Into<String>,
        severity: DiagnosticSeverity,
        message: impl Into<String>,
        target_field: impl Into<String>,
        points: Vec<SourcePoint>,
    ) {
        self.diagnostics.push(PendingDiagnostic {
            code: code.into(),
            severity,
            message: message.into(),
            target_field: target_field.into(),
            points,
        });
    }

    fn mark_used(&mut self, point: &SourcePoint) {
        self.used_artifact_ids.insert(point.artifact_id.clone());
    }
}

fn scan_document(
    document: &SourceDocument,
    state: &mut ScanState,
    limits: ProfilerLimits,
) -> Result<(), ProfilerError> {
    if let Value::Object(root) = &document.value {
        scan_metadata_object(root, "", document, state);
        if let Some(Value::Object(metadata)) = root.get("metadata") {
            scan_metadata_object(metadata, "/metadata", document, state);
        }
    }
    scan_value(&document.value, "", document, state, limits)
}

fn scan_metadata_object(
    object: &Map<String, Value>,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
) {
    if let Some(value) = object.get("profile_id").and_then(Value::as_str) {
        let point = SourcePoint::new(document, &join_pointer(pointer, "profile_id"));
        if valid_profile_id(value) {
            state.profile_ids.push(TextObservation {
                value: value.to_owned(),
                point: point.clone(),
            });
            state.mark_used(&point);
        } else {
            state.diagnostic(
                "invalid_profile_id",
                DiagnosticSeverity::Warning,
                format!("Ignored invalid structured profile_id {value:?}"),
                "/profile_id",
                vec![point],
            );
        }
    }
    for key in ["title", "study_name", "name"] {
        let Some(value) = object.get(key).and_then(Value::as_str) else {
            continue;
        };
        if valid_text(value) {
            let point = SourcePoint::new(document, &join_pointer(pointer, key));
            state.titles.push(TextObservation {
                value: value.to_owned(),
                point: point.clone(),
            });
            state.mark_used(&point);
            break;
        }
    }
}

fn scan_value(
    value: &Value,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
    limits: ProfilerLimits,
) -> Result<(), ProfilerError> {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                scan_value(
                    item,
                    &join_pointer(pointer, &index.to_string()),
                    document,
                    state,
                    limits,
                )?;
            }
        }
        Value::Object(object) => {
            scan_context(object, pointer, document, state);
            scan_entities(object, pointer, document, state);
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();
            for key in keys {
                let child_pointer = join_pointer(pointer, key);
                if key == "rules" {
                    scan_rule_container(&object[key], &child_pointer, document, state, limits)?;
                }
                scan_value(&object[key], &child_pointer, document, state, limits)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn scan_context(
    object: &Map<String, Value>,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
) {
    for key in CONTEXT_KEYS {
        let Some(value) = object.get(*key).and_then(Value::as_str) else {
            continue;
        };
        if !valid_text(value) || !is_biomedical_context(value) {
            continue;
        }
        let point = SourcePoint::new(document, &join_pointer(pointer, key));
        state.contexts.push(ContextObservation {
            key: (*key).to_owned(),
            value: value.to_owned(),
            point: point.clone(),
        });
        state.mark_used(&point);
    }
}

fn scan_entities(
    object: &Map<String, Value>,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
) {
    for key in INPUT_ID_KEYS {
        let Some(raw) = object.get(*key) else {
            continue;
        };
        let ids = structured_ids(raw, &join_pointer(pointer, key), document, state, "input");
        for (id, id_point) in ids {
            let observation = entity_observation(object, pointer, document, id_point.clone(), true);
            state.inputs.entry(id).or_default().push(observation);
            state.mark_used(&id_point);
        }
    }
    for key in OUTCOME_ID_KEYS {
        let Some(raw) = object.get(*key) else {
            continue;
        };
        let ids = structured_ids(raw, &join_pointer(pointer, key), document, state, "outcome");
        for (id, id_point) in ids {
            let observation =
                entity_observation(object, pointer, document, id_point.clone(), false);
            state.outcomes.entry(id).or_default().push(observation);
            state.mark_used(&id_point);
        }
    }
}

fn entity_observation(
    object: &Map<String, Value>,
    pointer: &str,
    document: &SourceDocument,
    id_point: SourcePoint,
    input: bool,
) -> EntityObservation {
    let label = ["label", "display_name", "name", "title"]
        .iter()
        .find_map(|key| {
            let value = object.get(*key)?.as_str()?;
            valid_text(value).then(|| TextObservation {
                value: value.to_owned(),
                point: SourcePoint::new(document, &join_pointer(pointer, key)),
            })
        });
    let numeric_key = if input { "cost" } else { "weight" };
    let number = object
        .get(numeric_key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| NumberObservation {
            value,
            point: SourcePoint::new(document, &join_pointer(pointer, numeric_key)),
        });
    let tags = object
        .get("tags")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |items| {
            items
                .iter()
                .enumerate()
                .filter_map(|(index, item)| {
                    let value = item.as_str()?;
                    valid_text(value).then(|| TextObservation {
                        value: value.to_owned(),
                        point: SourcePoint::new(
                            document,
                            &join_pointer(&join_pointer(pointer, "tags"), &index.to_string()),
                        ),
                    })
                })
                .collect()
        });
    EntityObservation {
        id_point,
        label,
        number,
        tags,
    }
}

fn structured_ids(
    value: &Value,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
    kind: &str,
) -> Vec<(String, SourcePoint)> {
    let mut raw_values = Vec::new();
    match value {
        Value::String(text) => raw_values.push((text.as_str(), pointer.to_owned())),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                if let Some(text) = item.as_str() {
                    raw_values.push((text, join_pointer(pointer, &index.to_string())));
                }
            }
        }
        _ => {}
    }
    let mut result = Vec::new();
    for (raw, value_pointer) in raw_values {
        let point = SourcePoint::new(document, &value_pointer);
        if valid_profile_id(raw) {
            result.push((raw.to_owned(), point));
        } else {
            state.diagnostic(
                "invalid_structured_id",
                DiagnosticSeverity::Warning,
                format!("Ignored invalid structured {kind} identity {raw:?}"),
                format!("/ignored/{kind}"),
                vec![point],
            );
        }
    }
    result
}

fn scan_rule_container(
    value: &Value,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
    limits: ProfilerLimits,
) -> Result<(), ProfilerError> {
    match value {
        Value::Array(rules) => {
            for (index, rule) in rules.iter().enumerate() {
                if let Value::Object(rule) = rule {
                    scan_rule(
                        rule,
                        &join_pointer(pointer, &index.to_string()),
                        document,
                        state,
                        limits,
                    )?;
                }
            }
        }
        Value::Object(rule_or_map) if rule_or_map.contains_key("conditions") => {
            scan_rule(rule_or_map, pointer, document, state, limits)?;
        }
        Value::Object(rule_map) => {
            let mut keys: Vec<_> = rule_map.keys().collect();
            keys.sort_unstable();
            for key in keys {
                if let Value::Object(rule) = &rule_map[key] {
                    scan_rule(rule, &join_pointer(pointer, key), document, state, limits)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn scan_rule(
    rule: &Map<String, Value>,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
    limits: ProfilerLimits,
) -> Result<(), ProfilerError> {
    state.rules_seen = state.rules_seen.saturating_add(1);
    if state.rules_seen > limits.max_rules {
        return Err(resource("structured rules", limits.max_rules));
    }
    let Some(conditions) = rule.get("conditions") else {
        state.diagnostic(
            "rule_without_conditions",
            DiagnosticSeverity::Warning,
            "Ignored structured rule without a conditions field",
            "/relationships",
            vec![SourcePoint::new(document, pointer)],
        );
        return Ok(());
    };

    let mut inputs = Vec::new();
    collect_condition_inputs(
        conditions,
        &join_pointer(pointer, "conditions"),
        document,
        state,
        &mut inputs,
    );
    let mut outcomes = Vec::new();
    let mut keys: Vec<_> = rule
        .keys()
        .filter(|key| key.as_str() != "conditions")
        .collect();
    keys.sort_unstable();
    for key in keys {
        collect_emissions(
            &rule[key],
            &join_pointer(pointer, key),
            Some(key),
            document,
            state,
            &mut outcomes,
        );
    }
    inputs.sort();
    inputs.dedup();
    outcomes.sort();
    outcomes.dedup();

    let rule_point = SourcePoint::new(document, pointer);
    if inputs.is_empty() {
        state.diagnostic(
            "rule_without_supported_input",
            DiagnosticSeverity::Warning,
            "Ignored rule relationship because conditions contained no supported input reference",
            "/relationships",
            vec![rule_point],
        );
        return Ok(());
    }
    if outcomes.is_empty() {
        state.diagnostic(
            "rule_without_supported_emission",
            DiagnosticSeverity::Warning,
            "Ignored rule relationship because it emitted no supported outcome or action",
            "/relationships",
            vec![rule_point],
        );
        return Ok(());
    }

    add_rule_relationships(&inputs, &outcomes, state, limits)
}

fn add_rule_relationships(
    inputs: &[(String, SourcePoint)],
    outcomes: &[(String, SourcePoint)],
    state: &mut ScanState,
    limits: ProfilerLimits,
) -> Result<(), ProfilerError> {
    for (input_id, input_point) in inputs {
        state
            .inputs
            .entry(input_id.clone())
            .or_default()
            .push(EntityObservation {
                id_point: input_point.clone(),
                label: None,
                number: None,
                tags: Vec::new(),
            });
        state.mark_used(input_point);
        for (outcome_id, outcome_point) in outcomes {
            state
                .outcomes
                .entry(outcome_id.clone())
                .or_default()
                .push(EntityObservation {
                    id_point: outcome_point.clone(),
                    label: None,
                    number: None,
                    tags: Vec::new(),
                });
            state.mark_used(outcome_point);
            let evidence = state
                .relationships
                .entry((input_id.clone(), outcome_id.clone()))
                .or_default();
            evidence.insert(input_point.clone());
            evidence.insert(outcome_point.clone());
            if state.relationships.len() > limits.max_relationships {
                return Err(resource(
                    "projected relationships",
                    limits.max_relationships,
                ));
            }
        }
    }
    Ok(())
}

fn collect_condition_inputs(
    value: &Value,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
    output: &mut Vec<(String, SourcePoint)>,
) {
    collect_condition_inputs_inner(value, pointer, document, state, output, true);
}

fn collect_condition_inputs_inner(
    value: &Value,
    pointer: &str,
    document: &SourceDocument,
    state: &mut ScanState,
    output: &mut Vec<(String, SourcePoint)>,
    allow_string_atom: bool,
) {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let item_pointer = join_pointer(pointer, &index.to_string());
                match (allow_string_atom, item) {
                    (true, Value::String(expression)) => {
                        collect_condition_expression(expression, &item_pointer, document, output);
                    }
                    _ => collect_condition_inputs_inner(
                        item,
                        &item_pointer,
                        document,
                        state,
                        output,
                        allow_string_atom,
                    ),
                }
            }
        }
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();
            for key in keys {
                let child_pointer = join_pointer(pointer, key);
                if CONDITION_INPUT_KEYS.contains(&key.as_str()) {
                    output.extend(structured_ids(
                        &object[key],
                        &child_pointer,
                        document,
                        state,
                        "condition input",
                    ));
                    continue;
                }
                let grouped_condition =
                    matches!(key.as_str(), "all_of" | "any_of" | "and" | "not" | "or");
                collect_condition_inputs_inner(
                    &object[key],
                    &child_pointer,
                    document,
                    state,
                    output,
                    grouped_condition,
                );
            }
        }
        Value::String(expression) if allow_string_atom => {
            collect_condition_expression(expression, pointer, document, output);
        }
        _ => {}
    }
}

fn collect_condition_expression(
    expression: &str,
    pointer: &str,
    document: &SourceDocument,
    output: &mut Vec<(String, SourcePoint)>,
) {
    if let Some(input_id) = condition_expression_input_id(expression) {
        output.push((input_id.to_owned(), SourcePoint::new(document, pointer)));
    }
}

/// Extract only the identifier from a deliberately tiny, non-executing
/// condition grammar: either one portable identifier or the left-hand side of
/// one comparison. Everything else remains unsupported rather than guessed.
fn condition_expression_input_id(expression: &str) -> Option<&str> {
    let expression = expression.trim();
    if valid_profile_id(expression) {
        return Some(expression);
    }
    for operator in ["==", "!=", ">=", "<=", ">", "<"] {
        let Some((left, right)) = expression.split_once(operator) else {
            continue;
        };
        let left = left.trim();
        if !right.trim().is_empty() && valid_profile_id(left) {
            return Some(left);
        }
    }
    None
}

fn collect_emissions(
    value: &Value,
    pointer: &str,
    key: Option<&str>,
    document: &SourceDocument,
    state: &mut ScanState,
    output: &mut Vec<(String, SourcePoint)>,
) {
    if key.is_some_and(|key| EMISSION_KEYS.contains(&key)) {
        output.extend(structured_ids(
            value,
            pointer,
            document,
            state,
            "emitted outcome",
        ));
    }
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_emissions(
                    item,
                    &join_pointer(pointer, &index.to_string()),
                    None,
                    document,
                    state,
                    output,
                );
            }
        }
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();
            for key in keys {
                collect_emissions(
                    &object[key],
                    &join_pointer(pointer, key),
                    Some(key),
                    document,
                    state,
                    output,
                );
            }
        }
        _ => {}
    }
}

fn build_heuristic_candidate(
    mut scan: ScanState,
    documents: &[&SourceDocument],
    context: &ProjectionContext,
    limits: ProfilerLimits,
) -> Result<ProjectionCandidate, ProfilerError> {
    scan.contexts.sort_by(|left, right| {
        left.point
            .cmp(&right.point)
            .then_with(|| left.key.cmp(&right.key))
            .then_with(|| left.value.cmp(&right.value))
    });
    scan.profile_ids.sort_by(text_observation_order);
    scan.titles.sort_by(text_observation_order);
    let anchor = scan.contexts[0].point.clone();
    let mut builder = CandidateBuilder::default();
    for diagnostic in &scan.diagnostics {
        builder.add_diagnostic(
            &diagnostic.code,
            diagnostic.severity,
            &diagnostic.message,
            &diagnostic.target_field,
            &diagnostic.points,
        );
    }

    let profile_id = choose_profile_id(&scan, context, &anchor, &mut builder);
    let (title, biomedical_scope) = build_scope(&scan, context, &anchor, &mut builder);
    let (inputs, outcomes, relationships) = build_collections(&scan, &mut builder);

    let constraints = default_constraints(&mut builder, &anchor);
    builder.add_default(
        "/objective",
        "default_objective",
        "maximize_weighted_coverage",
        "The built-in heuristic uses the benchmark kernel's sole supported primary objective",
        std::slice::from_ref(&anchor),
    );

    let provenance = build_provenance(documents, context, &anchor, &mut builder);

    let profile = BenchmarkProfile {
        schema_version: BENCHMARK_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id,
        title,
        biomedical_scope,
        inputs,
        outcomes,
        relationships,
        constraints,
        objective: BenchmarkObjective::MaximizeWeightedCoverage,
        provenance,
    };
    profile.validate().map_err(ProfilerError::InvalidProfile)?;

    builder.add_diagnostic(
        "approval_required",
        DiagnosticSeverity::Info,
        "Structured biomedical projection is heuristic and must be reviewed before optimization",
        "/profile",
        std::slice::from_ref(&anchor),
    );
    let confidence_bps = heuristic_confidence(&profile, &builder);
    let (extracted_fields, defaulted_fields, evidence, diagnostics, assumptions) =
        builder.finish(limits)?;
    let candidate = ProjectionCandidate {
        schema_version: PROJECTION_CANDIDATE_SCHEMA_VERSION.to_owned(),
        method: ProjectionMethod::BiomedicalHeuristic,
        status: ProjectionStatus::RequiresApproval,
        confidence_bps,
        profile,
        extracted_fields,
        defaulted_fields,
        evidence,
        diagnostics,
        assumptions,
    };
    candidate.validate()?;
    Ok(candidate)
}

fn build_scope(
    scan: &ScanState,
    context: &ProjectionContext,
    anchor: &SourcePoint,
    builder: &mut CandidateBuilder,
) -> (String, BiomedicalScope) {
    let population = choose_population(scan, context, anchor, builder);
    let title = choose_title(scan, context, &population, anchor, builder);
    let area = choose_area(scan, context, anchor, builder);
    let scope = BiomedicalScope {
        area,
        population,
        input_semantics: default_text(
            builder,
            "/biomedical_scope/input_semantics",
            "biomarker, assay, parameter, or biomedical feature",
            "default_input_semantics",
            "The supported structured input identifiers are interpreted as candidate biomedical measurements",
            anchor,
        ),
        outcome_semantics: default_text(
            builder,
            "/biomedical_scope/outcome_semantics",
            "covered clinical action, option, or biomedical outcome",
            "default_outcome_semantics",
            "The supported rule emissions are interpreted as coverable biomedical outcomes",
            anchor,
        ),
    };
    (title, scope)
}

fn build_collections(
    scan: &ScanState,
    builder: &mut CandidateBuilder,
) -> (
    Vec<BenchmarkInput>,
    Vec<BenchmarkOutcome>,
    Vec<IncidenceRelationship>,
) {
    let inputs = scan
        .inputs
        .iter()
        .map(|(id, observations)| build_input(id, observations, builder))
        .collect();
    let outcomes = scan
        .outcomes
        .iter()
        .map(|(id, observations)| build_outcome(id, observations, builder))
        .collect();
    let relationships = scan
        .relationships
        .iter()
        .map(|((input_id, outcome_id), points)| {
            let target = relationship_target(input_id, outcome_id);
            builder.extracted_fields.insert(target.clone());
            builder.add_evidence(&target, &points.iter().cloned().collect::<Vec<_>>());
            IncidenceRelationship {
                input_id: input_id.clone(),
                outcome_id: outcome_id.clone(),
            }
        })
        .collect();
    (inputs, outcomes, relationships)
}

fn build_provenance(
    documents: &[&SourceDocument],
    context: &ProjectionContext,
    anchor: &SourcePoint,
    builder: &mut CandidateBuilder,
) -> ProfileProvenance {
    let provenance_points: Vec<_> = documents
        .iter()
        .map(|document| SourcePoint::new(document, ""))
        .collect();
    builder
        .extracted_fields
        .insert("/provenance/source_artifact_ids".to_owned());
    builder.add_evidence("/provenance/source_artifact_ids", &provenance_points);
    let generated_by = context
        .generated_by
        .clone()
        .unwrap_or_else(|| concat!("qbm-profiler/", env!("CARGO_PKG_VERSION")).to_owned());
    let (generator_code, generator_message) = if context.generated_by.is_some() {
        (
            "caller_projection_generator",
            "The integration supplied the projection generator identity",
        )
    } else {
        (
            "default_projection_generator",
            "The profiler records its own package version as the projection generator",
        )
    };
    builder.add_default(
        "/provenance/generated_by",
        generator_code,
        &generated_by,
        generator_message,
        std::slice::from_ref(anchor),
    );
    let (revision_code, revision_value, revision_message) =
        context.source_revision.as_ref().map_or(
            (
                "missing_source_revision",
                "null",
                "No immutable source revision was supplied",
            ),
            |revision| {
                (
                    "caller_source_revision",
                    revision.as_str(),
                    "The integration supplied the immutable source revision",
                )
            },
        );
    builder.add_default(
        "/provenance/source_revision",
        revision_code,
        revision_value,
        revision_message,
        std::slice::from_ref(anchor),
    );
    let projection_method = "conservative structured biomedical heuristic; human approval required";
    builder.add_default(
        "/provenance/projection_method",
        "default_projection_method",
        projection_method,
        "The candidate was created by the fixed built-in heuristic",
        std::slice::from_ref(anchor),
    );
    ProfileProvenance {
        generated_by,
        source_revision: context.source_revision.clone(),
        source_artifact_ids: documents
            .iter()
            .map(|document| document.artifact_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        projection_method: projection_method.to_owned(),
    }
}

fn choose_profile_id(
    scan: &ScanState,
    context: &ProjectionContext,
    anchor: &SourcePoint,
    builder: &mut CandidateBuilder,
) -> String {
    if let Some(profile_id) = &context.profile_id {
        builder.add_default(
            "/profile_id",
            "caller_profile_id",
            profile_id,
            "The integration supplied a stable logical identity for cross-version comparison",
            std::slice::from_ref(anchor),
        );
        return profile_id.clone();
    }
    if let Some(selected) = scan.profile_ids.first() {
        let target = "/profile_id";
        builder.extracted_fields.insert(target.to_owned());
        builder.add_evidence(target, std::slice::from_ref(&selected.point));
        diagnose_text_conflict("profile_id", target, &scan.profile_ids, builder);
        return selected.value.clone();
    }
    let profile_id = logical_profile_id(&anchor.path);
    builder.add_default(
        "/profile_id",
        "default_profile_id",
        &profile_id,
        "No valid structured profile_id was supplied; identity was derived from the stable logical source path, never artifact content",
        std::slice::from_ref(anchor),
    );
    profile_id
}

fn choose_title(
    scan: &ScanState,
    context: &ProjectionContext,
    population: &str,
    anchor: &SourcePoint,
    builder: &mut CandidateBuilder,
) -> String {
    if let Some(title) = &context.title {
        builder.add_default(
            "/title",
            "caller_profile_title",
            title,
            "The integration supplied the profile title",
            std::slice::from_ref(anchor),
        );
        return title.clone();
    }
    if let Some(selected) = scan.titles.first() {
        let target = "/title";
        builder.extracted_fields.insert(target.to_owned());
        builder.add_evidence(target, std::slice::from_ref(&selected.point));
        diagnose_text_conflict("title", target, &scan.titles, builder);
        return selected.value.clone();
    }
    let proposed = format!("{population} biomedical benchmark profile");
    let title = if proposed.len() <= 4_096 {
        proposed
    } else {
        "Biomedical benchmark profile".to_owned()
    };
    builder.add_default(
        "/title",
        "default_profile_title",
        &title,
        "No supported structured title was supplied",
        std::slice::from_ref(anchor),
    );
    title
}

fn choose_population(
    scan: &ScanState,
    context: &ProjectionContext,
    anchor: &SourcePoint,
    builder: &mut CandidateBuilder,
) -> String {
    if let Some(population) = &context.population {
        builder.add_default(
            "/biomedical_scope/population",
            "caller_population",
            population,
            "The integration supplied the reviewed population boundary",
            std::slice::from_ref(anchor),
        );
        return population.clone();
    }
    if let Some(selected) = scan
        .contexts
        .iter()
        .find(|observation| POPULATION_KEYS.contains(&observation.key.as_str()))
    {
        let target = "/biomedical_scope/population";
        builder.extracted_fields.insert(target.to_owned());
        builder.add_evidence(target, std::slice::from_ref(&selected.point));
        return selected.value.clone();
    }
    default_text(
        builder,
        "/biomedical_scope/population",
        "Population described by the reviewed biomedical source",
        "default_population",
        "No supported population, cohort, indication, disease, or cancer_type value was supplied",
        anchor,
    )
}

fn choose_area(
    scan: &ScanState,
    context: &ProjectionContext,
    anchor: &SourcePoint,
    builder: &mut CandidateBuilder,
) -> String {
    if let Some(area) = &context.biomedical_area {
        builder.add_default(
            "/biomedical_scope/area",
            "caller_biomedical_area",
            area,
            "The integration supplied the reviewed biomedical area",
            std::slice::from_ref(anchor),
        );
        return area.clone();
    }
    if let Some(selected) = scan
        .contexts
        .iter()
        .find(|observation| AREA_KEYS.contains(&observation.key.as_str()))
    {
        let target = "/biomedical_scope/area";
        builder.extracted_fields.insert(target.to_owned());
        builder.add_evidence(target, std::slice::from_ref(&selected.point));
        return selected.value.clone();
    }
    let area = if scan
        .contexts
        .iter()
        .any(|observation| is_oncology_context(&observation.value))
    {
        "oncology"
    } else {
        "biomedical"
    };
    default_text(
        builder,
        "/biomedical_scope/area",
        area,
        "default_biomedical_area",
        "The area was conservatively classified from a supported structured biomedical context value",
        anchor,
    )
}

fn default_text(
    builder: &mut CandidateBuilder,
    target: &str,
    value: &str,
    code: &str,
    message: &str,
    anchor: &SourcePoint,
) -> String {
    builder.add_default(target, code, value, message, std::slice::from_ref(anchor));
    value.to_owned()
}

fn build_input(
    id: &str,
    observations: &[EntityObservation],
    builder: &mut CandidateBuilder,
) -> BenchmarkInput {
    let prefix = format!("/inputs/{}/", escape_pointer_segment(id));
    let id_target = format!("{prefix}id");
    let id_points = unique_id_points(observations);
    builder.extracted_fields.insert(id_target.clone());
    builder.add_evidence(&id_target, &id_points);
    let label = choose_entity_label(id, observations, &prefix, builder);
    let cost = choose_entity_number(id, observations, &prefix, true, builder);
    let tags = choose_entity_tags(observations, &prefix, builder);
    BenchmarkInput {
        id: id.to_owned(),
        label,
        cost,
        tags,
    }
}

fn build_outcome(
    id: &str,
    observations: &[EntityObservation],
    builder: &mut CandidateBuilder,
) -> BenchmarkOutcome {
    let prefix = format!("/outcomes/{}/", escape_pointer_segment(id));
    let id_target = format!("{prefix}id");
    let id_points = unique_id_points(observations);
    builder.extracted_fields.insert(id_target.clone());
    builder.add_evidence(&id_target, &id_points);
    let label = choose_entity_label(id, observations, &prefix, builder);
    let weight = choose_entity_number(id, observations, &prefix, false, builder);
    let tags = choose_entity_tags(observations, &prefix, builder);
    BenchmarkOutcome {
        id: id.to_owned(),
        label,
        weight,
        tags,
    }
}

fn choose_entity_label(
    id: &str,
    observations: &[EntityObservation],
    prefix: &str,
    builder: &mut CandidateBuilder,
) -> String {
    let target = format!("{prefix}label");
    let mut labels: Vec<_> = observations
        .iter()
        .filter_map(|observation| observation.label.as_ref())
        .collect();
    labels.sort_by(|left, right| text_observation_order(left, right));
    if let Some(selected) = labels.first() {
        builder.extracted_fields.insert(target.clone());
        builder.add_evidence(&target, std::slice::from_ref(&selected.point));
        let distinct: BTreeSet<_> = labels.iter().map(|item| item.value.as_str()).collect();
        if distinct.len() > 1 {
            let points = labels
                .iter()
                .map(|item| item.point.clone())
                .collect::<Vec<_>>();
            builder.add_diagnostic(
                "conflicting_entity_label",
                DiagnosticSeverity::Warning,
                &format!(
                    "Conflicting labels for {id:?}; selected the value with earliest deterministic evidence"
                ),
                &target,
                &points,
            );
        }
        return selected.value.clone();
    }
    builder.add_default(
        &target,
        "default_entity_label",
        id,
        "No supported structured label was supplied; the stable identity is used as the label",
        &unique_id_points(observations),
    );
    id.to_owned()
}

fn choose_entity_number(
    id: &str,
    observations: &[EntityObservation],
    prefix: &str,
    input: bool,
    builder: &mut CandidateBuilder,
) -> f64 {
    let field = if input { "cost" } else { "weight" };
    let target = format!("{prefix}{field}");
    let mut values: Vec<_> = observations
        .iter()
        .filter_map(|observation| observation.number.as_ref())
        .collect();
    values.sort_by(|left, right| {
        left.point
            .cmp(&right.point)
            .then_with(|| left.value.total_cmp(&right.value))
    });
    if let Some(selected) = values.first() {
        builder.extracted_fields.insert(target.clone());
        builder.add_evidence(&target, std::slice::from_ref(&selected.point));
        if values
            .iter()
            .any(|item| item.value.total_cmp(&selected.value) != Ordering::Equal)
        {
            let points = values
                .iter()
                .map(|item| item.point.clone())
                .collect::<Vec<_>>();
            builder.add_diagnostic(
                "conflicting_entity_number",
                DiagnosticSeverity::Warning,
                &format!(
                    "Conflicting {field} values for {id:?}; selected the value with earliest deterministic evidence"
                ),
                &target,
                &points,
            );
        }
        return selected.value;
    }
    let code = if input {
        "default_input_cost"
    } else {
        "default_outcome_weight"
    };
    builder.add_default(
        &target,
        code,
        "1",
        &format!(
            "No positive finite structured {field} was supplied for {id:?}; value 1 requires reviewer confirmation"
        ),
        &unique_id_points(observations),
    );
    1.0
}

fn choose_entity_tags(
    observations: &[EntityObservation],
    prefix: &str,
    builder: &mut CandidateBuilder,
) -> Vec<String> {
    let target = format!("{prefix}tags");
    let mut by_tag: BTreeMap<String, Vec<SourcePoint>> = BTreeMap::new();
    for tag in observations
        .iter()
        .flat_map(|observation| &observation.tags)
    {
        by_tag
            .entry(tag.value.clone())
            .or_default()
            .push(tag.point.clone());
    }
    if by_tag.is_empty() {
        builder.add_default(
            &target,
            "default_empty_tags",
            "[]",
            "No supported structured tags were supplied",
            &unique_id_points(observations),
        );
        return Vec::new();
    }
    builder.extracted_fields.insert(target.clone());
    let mut points = Vec::new();
    for tag_points in by_tag.values_mut() {
        tag_points.sort();
        tag_points.dedup();
        points.extend(tag_points.iter().cloned());
    }
    builder.add_evidence(&target, &points);
    by_tag.into_keys().collect()
}

fn unique_id_points(observations: &[EntityObservation]) -> Vec<SourcePoint> {
    observations
        .iter()
        .map(|observation| observation.id_point.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn default_constraints(
    builder: &mut CandidateBuilder,
    anchor: &SourcePoint,
) -> BenchmarkConstraints {
    for (target, code, value, message) in [
        (
            "/constraints/min_selected",
            "default_min_selected",
            "0",
            "No supported feasibility projection is performed; selection may start at zero inputs",
        ),
        (
            "/constraints/max_selected",
            "default_max_selected",
            "null",
            "No maximum panel size was inferred",
        ),
        (
            "/constraints/max_total_cost",
            "default_max_total_cost",
            "null",
            "No cost budget was inferred",
        ),
        (
            "/constraints/required_inputs",
            "default_required_inputs",
            "[]",
            "No required inputs were inferred",
        ),
        (
            "/constraints/excluded_inputs",
            "default_excluded_inputs",
            "[]",
            "No excluded inputs were inferred",
        ),
        (
            "/constraints/required_outcomes",
            "default_required_outcomes",
            "[]",
            "No required outcomes were inferred",
        ),
    ] {
        builder.add_default(target, code, value, message, std::slice::from_ref(anchor));
    }
    BenchmarkConstraints {
        min_selected: 0,
        max_selected: None,
        max_total_cost: None,
        required_inputs: Vec::new(),
        excluded_inputs: Vec::new(),
        required_outcomes: Vec::new(),
    }
}

fn diagnose_text_conflict(
    name: &str,
    target: &str,
    observations: &[TextObservation],
    builder: &mut CandidateBuilder,
) {
    let distinct: BTreeSet<_> = observations
        .iter()
        .map(|item| item.value.as_str())
        .collect();
    if distinct.len() > 1 {
        let points = observations
            .iter()
            .map(|item| item.point.clone())
            .collect::<Vec<_>>();
        builder.add_diagnostic(
            &format!("conflicting_{name}"),
            DiagnosticSeverity::Warning,
            &format!(
                "Conflicting {name} values; selected the value with earliest deterministic evidence"
            ),
            target,
            &points,
        );
    }
}

fn heuristic_confidence(profile: &BenchmarkProfile, builder: &CandidateBuilder) -> u16 {
    let mut confidence = 5_500_u16;
    if profile.biomedical_scope.population
        != "Population described by the reviewed biomedical source"
    {
        confidence = confidence.saturating_add(500);
    }
    if !builder
        .assumptions
        .iter()
        .any(|item| item.code == "default_input_cost")
    {
        confidence = confidence.saturating_add(500);
    }
    if !builder
        .assumptions
        .iter()
        .any(|item| item.code == "default_outcome_weight")
    {
        confidence = confidence.saturating_add(500);
    }
    confidence = confidence
        .saturating_add(u16::try_from(profile.relationships.len().min(10)).unwrap_or(10) * 100);
    if builder
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    {
        confidence = confidence.saturating_sub(750);
    }
    confidence.min(8_500)
}

fn text_observation_order(left: &TextObservation, right: &TextObservation) -> Ordering {
    left.point
        .cmp(&right.point)
        .then_with(|| left.value.cmp(&right.value))
}

fn relationship_target(input_id: &str, outcome_id: &str) -> String {
    format!(
        "/relationships/{}->{}",
        escape_pointer_segment(input_id),
        escape_pointer_segment(outcome_id)
    )
}

#[derive(Debug, Default)]
struct CandidateBuilder {
    extracted_fields: BTreeSet<String>,
    defaulted_fields: BTreeSet<String>,
    evidence: BTreeSet<ProjectionEvidence>,
    diagnostics: BTreeSet<ProjectionDiagnostic>,
    assumptions: BTreeSet<ProjectionAssumption>,
}

impl CandidateBuilder {
    fn add_evidence(&mut self, target: &str, points: &[SourcePoint]) -> Vec<ProjectionEvidence> {
        let records: Vec<_> = points
            .iter()
            .map(|point| point.to_evidence(target))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self.evidence.extend(records.iter().cloned());
        records
    }

    fn add_diagnostic(
        &mut self,
        code: &str,
        severity: DiagnosticSeverity,
        message: &str,
        target: &str,
        points: &[SourcePoint],
    ) {
        let evidence = self.add_evidence(target, points);
        self.diagnostics.insert(ProjectionDiagnostic {
            code: code.to_owned(),
            severity,
            message: message.to_owned(),
            evidence,
        });
    }

    fn add_default(
        &mut self,
        target: &str,
        code: &str,
        value: &str,
        message: &str,
        points: &[SourcePoint],
    ) {
        self.defaulted_fields.insert(target.to_owned());
        let evidence = self.add_evidence(target, points);
        self.assumptions.insert(ProjectionAssumption {
            code: code.to_owned(),
            target_field: target.to_owned(),
            value: value.to_owned(),
            message: message.to_owned(),
            evidence,
        });
    }

    fn finish(self, limits: ProfilerLimits) -> Result<FinishedCandidateParts, ProfilerError> {
        if self.evidence.len() > limits.max_evidence {
            return Err(resource("projection evidence", limits.max_evidence));
        }
        Ok((
            self.extracted_fields.into_iter().collect(),
            self.defaulted_fields.into_iter().collect(),
            self.evidence.into_iter().collect(),
            self.diagnostics.into_iter().collect(),
            self.assumptions.into_iter().collect(),
        ))
    }
}

type FinishedCandidateParts = (
    Vec<String>,
    Vec<String>,
    Vec<ProjectionEvidence>,
    Vec<ProjectionDiagnostic>,
    Vec<ProjectionAssumption>,
);

#[derive(Debug, Default)]
struct ValueBudget {
    elements: usize,
    string_bytes: usize,
}

fn measure_value(
    value: &Value,
    depth: usize,
    budget: &mut ValueBudget,
    limits: ProfilerLimits,
) -> Result<(), ProfilerError> {
    if depth > limits.max_value_depth {
        return Err(resource("value depth", limits.max_value_depth));
    }
    add_budget(
        &mut budget.elements,
        1,
        limits.max_value_elements,
        "value elements",
    )?;
    match value {
        Value::String(text) => add_budget(
            &mut budget.string_bytes,
            text.len(),
            limits.max_total_string_bytes,
            "string bytes",
        )?,
        Value::Array(items) => {
            for item in items {
                measure_value(item, depth.saturating_add(1), budget, limits)?;
            }
        }
        Value::Object(object) => {
            for (key, child) in object {
                add_budget(
                    &mut budget.elements,
                    1,
                    limits.max_value_elements,
                    "value elements",
                )?;
                add_budget(
                    &mut budget.string_bytes,
                    key.len(),
                    limits.max_total_string_bytes,
                    "string bytes",
                )?;
                measure_value(child, depth.saturating_add(1), budget, limits)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn add_budget(
    current: &mut usize,
    amount: usize,
    maximum: usize,
    kind: &'static str,
) -> Result<(), ProfilerError> {
    *current = current
        .checked_add(amount)
        .ok_or_else(|| resource(kind, maximum))?;
    if *current > maximum {
        return Err(resource(kind, maximum));
    }
    Ok(())
}

fn validate_source_metadata(document: &SourceDocument) -> Result<(), ProfilerError> {
    if document.path.is_empty()
        || document.path.len() > 4_096
        || document.path.starts_with('/')
        || document.path.contains('\\')
        || document.path.chars().any(char::is_control)
        || document
            .path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(document_error(
            document,
            "path must be a normalized, control-free relative path of at most 4096 bytes",
        ));
    }
    if !valid_profile_id(&document.artifact_id) {
        return Err(document_error(
            document,
            "artifact_id must contain 1..=256 ASCII letters, digits, '.', '_', ':', '/', or '-'",
        ));
    }
    Ok(())
}

fn validate_projection_context(context: &ProjectionContext) -> Result<(), ProfilerError> {
    if let Some(profile_id) = &context.profile_id {
        if !valid_profile_id(profile_id) {
            return Err(context_error(
                "profile_id",
                "must contain 1..=256 ASCII letters, digits, '.', '_', ':', '/', or '-'",
            ));
        }
    }
    for (field, value) in [
        ("title", context.title.as_deref()),
        ("biomedical_area", context.biomedical_area.as_deref()),
        ("population", context.population.as_deref()),
        ("source_revision", context.source_revision.as_deref()),
        ("generated_by", context.generated_by.as_deref()),
    ] {
        if value.is_some_and(|value| !valid_text(value)) {
            return Err(context_error(
                field,
                "must be non-empty, trimmed, control-free, and at most 4096 bytes",
            ));
        }
    }
    Ok(())
}

fn logical_profile_id(path: &str) -> String {
    let mut identity = String::from("profile/");
    let mut previous_was_separator = false;
    for character in path.chars() {
        let normalized = if character.is_ascii_alphanumeric()
            || matches!(character, '.' | '_' | ':' | '/' | '-')
        {
            character
        } else {
            '-'
        };
        if normalized == '-' && previous_was_separator {
            continue;
        }
        if identity.len() == 256 {
            break;
        }
        identity.push(normalized);
        previous_was_separator = normalized == '-';
    }
    identity
}

fn valid_profile_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4_096
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn is_biomedical_context(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    [
        "biomedical",
        "biomedicine",
        "clinical",
        "oncology",
        "cancer",
        "tumor",
        "tumour",
        "carcinoma",
        "leukemia",
        "leukaemia",
        "lymphoma",
        "melanoma",
        "sarcoma",
        "nsclc",
    ]
    .iter()
    .any(|term| normalized.contains(term))
}

fn is_oncology_context(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    [
        "oncology",
        "cancer",
        "tumor",
        "tumour",
        "carcinoma",
        "leukemia",
        "leukaemia",
        "lymphoma",
        "melanoma",
        "sarcoma",
        "nsclc",
    ]
    .iter()
    .any(|term| normalized.contains(term))
}

fn join_pointer(base: &str, segment: &str) -> String {
    format!("{base}/{}", escape_pointer_segment(segment))
}

fn escape_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn resource(kind: &'static str, maximum: usize) -> ProfilerError {
    ProfilerError::ResourceLimit { kind, maximum }
}

fn document_error(document: &SourceDocument, message: impl Into<String>) -> ProfilerError {
    ProfilerError::InvalidDocument {
        path: document.path.clone(),
        message: message.into(),
    }
}

fn context_error(field: &'static str, message: impl Into<String>) -> ProfilerError {
    ProfilerError::InvalidContext {
        field,
        message: message.into(),
    }
}
