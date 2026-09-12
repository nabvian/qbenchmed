//! Schema invariant validation.

use std::collections::BTreeSet;

use crate::coverage::CoverageModel;
use crate::{
    BENCHMARK_PROFILE_SCHEMA_V1, BenchmarkError, BenchmarkProfile, RelationshipKind,
    SUPPORTED_PROFILE_SCHEMA_VERSIONS,
};

#[allow(clippy::too_many_lines)] // Keeping schema invariants together makes the validation contract auditable.
pub(crate) fn validate_profile(profile: &BenchmarkProfile) -> Result<(), BenchmarkError> {
    if !SUPPORTED_PROFILE_SCHEMA_VERSIONS.contains(&profile.schema_version.as_str()) {
        return invalid(format!(
            "unsupported schema_version {:?}; expected one of {}",
            profile.schema_version,
            SUPPORTED_PROFILE_SCHEMA_VERSIONS.join(", ")
        ));
    }
    // A v1 document is welcome, but it has to mean v1. Silently upgrading one
    // that carries typed edges would make the declared version a lie, and the
    // version is what a reader uses to know which coverage rule applies.
    if profile.schema_version == BENCHMARK_PROFILE_SCHEMA_V1 && profile.uses_typed_features() {
        return invalid(
            "this document declares schema v1 but uses relationship kinds, path groups, context inputs, or unconditional outcomes; declare v2 instead",
        );
    }
    validate_id("profile_id", &profile.profile_id)?;
    validate_text("title", &profile.title)?;
    validate_text("biomedical_scope.area", &profile.biomedical_scope.area)?;
    validate_text(
        "biomedical_scope.population",
        &profile.biomedical_scope.population,
    )?;
    validate_text(
        "biomedical_scope.input_semantics",
        &profile.biomedical_scope.input_semantics,
    )?;
    validate_text(
        "biomedical_scope.outcome_semantics",
        &profile.biomedical_scope.outcome_semantics,
    )?;

    if profile.inputs.is_empty() {
        return invalid("inputs must not be empty");
    }
    if profile.outcomes.is_empty() {
        return invalid("outcomes must not be empty");
    }
    ensure_sorted_unique(
        "inputs",
        profile.inputs.iter().map(|input| input.id.as_str()),
    )?;
    ensure_sorted_unique(
        "outcomes",
        profile.outcomes.iter().map(|outcome| outcome.id.as_str()),
    )?;

    for input in &profile.inputs {
        validate_id("input.id", &input.id)?;
        validate_text("input.label", &input.label)?;
        validate_positive("input.cost", input.cost)?;
        validate_tags("input.tags", &input.tags)?;
    }
    for outcome in &profile.outcomes {
        validate_id("outcome.id", &outcome.id)?;
        validate_text("outcome.label", &outcome.label)?;
        validate_positive("outcome.weight", outcome.weight)?;
        validate_tags("outcome.tags", &outcome.tags)?;
    }

    let input_ids: BTreeSet<_> = profile
        .inputs
        .iter()
        .map(|input| input.id.as_str())
        .collect();
    let outcome_ids: BTreeSet<_> = profile
        .outcomes
        .iter()
        .map(|outcome| outcome.id.as_str())
        .collect();

    if !profile
        .relationships
        .windows(2)
        .all(|pair| pair[0] < pair[1])
    {
        return invalid("relationships must be strictly sorted and unique");
    }
    for relationship in &profile.relationships {
        if !input_ids.contains(relationship.input_id.as_str()) {
            return invalid(format!(
                "relationship references unknown input {:?}",
                relationship.input_id
            ));
        }
        if !outcome_ids.contains(relationship.outcome_id.as_str()) {
            return invalid(format!(
                "relationship references unknown outcome {:?}",
                relationship.outcome_id
            ));
        }
        if !relationship.path.is_empty() {
            validate_path("relationship.path", &relationship.path)?;
        }
        match (&relationship.context_input_id, relationship.kind) {
            (Some(context), RelationshipKind::Contextual) => {
                if !input_ids.contains(context.as_str()) {
                    return invalid(format!(
                        "relationship context_input_id references unknown input {context:?}"
                    ));
                }
                if context == &relationship.input_id {
                    return invalid(format!(
                        "relationship for input {:?} cannot be its own context",
                        relationship.input_id
                    ));
                }
            }
            (Some(_), other) => {
                return invalid(format!(
                    "context_input_id is only meaningful on a contextual relationship, not {}",
                    other.as_str()
                ));
            }
            (None, _) => {}
        }
    }

    validate_constraint_ids(
        "unconditional_outcomes",
        &profile.unconditional_outcomes,
        &outcome_ids,
    )?;

    validate_constraint_ids(
        "required_inputs",
        &profile.constraints.required_inputs,
        &input_ids,
    )?;
    validate_constraint_ids(
        "excluded_inputs",
        &profile.constraints.excluded_inputs,
        &input_ids,
    )?;
    validate_constraint_ids(
        "required_outcomes",
        &profile.constraints.required_outcomes,
        &outcome_ids,
    )?;

    let required: BTreeSet<_> = profile
        .constraints
        .required_inputs
        .iter()
        .map(String::as_str)
        .collect();
    let excluded: BTreeSet<_> = profile
        .constraints
        .excluded_inputs
        .iter()
        .map(String::as_str)
        .collect();
    if let Some(overlap) = required.intersection(&excluded).next() {
        return invalid(format!(
            "input {overlap:?} cannot be both required and excluded"
        ));
    }

    let available = profile.inputs.len() - excluded.len();
    if profile.constraints.min_selected > available {
        return invalid("min_selected exceeds the number of non-excluded inputs");
    }
    if let Some(maximum) = profile.constraints.max_selected {
        if maximum > profile.inputs.len() {
            return invalid("max_selected exceeds the number of inputs");
        }
        if maximum < profile.constraints.min_selected {
            return invalid("max_selected is below min_selected");
        }
        if required.len() > maximum {
            return invalid("required_inputs contains more inputs than max_selected");
        }
    }
    if let Some(maximum_cost) = profile.constraints.max_total_cost {
        validate_positive("constraints.max_total_cost", maximum_cost)?;
        let required_cost: f64 = profile
            .inputs
            .iter()
            .filter(|input| required.contains(input.id.as_str()))
            .map(|input| input.cost)
            .sum();
        if required_cost > maximum_cost {
            return invalid("required input cost exceeds max_total_cost");
        }
    }

    // Reachability has to be decided by the typed rules, not by "some
    // relationship mentions it": an outcome whose only arm needs two inputs
    // together is unreachable the moment either one is excluded, even though a
    // raw incidence lookup would happily find a row for it.
    if !profile.constraints.required_outcomes.is_empty() {
        let model = CoverageModel::compile(profile)?;
        let mut allowed = crate::coverage::InputSet::empty(profile.inputs.len());
        for (index, input) in profile.inputs.iter().enumerate() {
            if !excluded.contains(input.id.as_str()) {
                allowed.insert(index);
            }
        }
        for required in &profile.constraints.required_outcomes {
            let position = profile
                .outcomes
                .iter()
                .position(|outcome| &outcome.id == required)
                .ok_or_else(|| {
                    BenchmarkError::InvalidProfile(format!(
                        "required outcome {required:?} is unknown"
                    ))
                })?;
            if !model.is_reachable_within(position, &allowed) {
                return invalid(format!(
                    "required outcome {required:?} is unreachable using non-excluded inputs"
                ));
            }
        }
    }

    validate_text("provenance.generated_by", &profile.provenance.generated_by)?;
    if let Some(revision) = &profile.provenance.source_revision {
        validate_text("provenance.source_revision", revision)?;
    }
    ensure_sorted_unique(
        "provenance.source_artifact_ids",
        profile
            .provenance
            .source_artifact_ids
            .iter()
            .map(String::as_str),
    )?;
    if profile.provenance.source_artifact_ids.is_empty() {
        return invalid("provenance.source_artifact_ids must not be empty");
    }
    for artifact_id in &profile.provenance.source_artifact_ids {
        validate_id("provenance.source_artifact_ids", artifact_id)?;
    }
    validate_text(
        "provenance.projection_method",
        &profile.provenance.projection_method,
    )?;

    Ok(())
}

fn validate_constraint_ids(
    field: &str,
    values: &[String],
    known: &BTreeSet<&str>,
) -> Result<(), BenchmarkError> {
    ensure_sorted_unique(field, values.iter().map(String::as_str))?;
    if let Some(unknown) = values.iter().find(|value| !known.contains(value.as_str())) {
        return invalid(format!("{field} references unknown identity {unknown:?}"));
    }
    Ok(())
}

fn validate_tags(field: &str, tags: &[String]) -> Result<(), BenchmarkError> {
    ensure_sorted_unique(field, tags.iter().map(String::as_str))?;
    for tag in tags {
        validate_text(field, tag)?;
    }
    Ok(())
}

fn ensure_sorted_unique<'a>(
    field: &str,
    values: impl Iterator<Item = &'a str>,
) -> Result<(), BenchmarkError> {
    let values: Vec<_> = values.collect();
    if !values.windows(2).all(|pair| pair[0] < pair[1]) {
        return invalid(format!("{field} must be strictly sorted and unique"));
    }
    Ok(())
}

fn validate_id(field: &str, value: &str) -> Result<(), BenchmarkError> {
    if value.is_empty() || value.len() > 256 {
        return invalid(format!("{field} must contain 1..=256 bytes"));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
    }) {
        return invalid(format!(
            "{field} must use ASCII letters, digits, '.', '_', ':', '/', or '-'"
        ));
    }
    Ok(())
}

/// Path labels follow the identifier grammar plus `#`.
///
/// Exporters conventionally name an arm after the outcome it belongs to,
/// `IDA#ferritin_crp`, and rejecting that separator would force every exporter
/// to rewrite labels that trace back to numbered audit rows.
fn validate_path(field: &str, value: &str) -> Result<(), BenchmarkError> {
    if value.is_empty() || value.len() > 256 {
        return invalid(format!("{field} must contain 1..=256 bytes"));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-' | b'#')
    }) {
        return invalid(format!(
            "{field} must use ASCII letters, digits, '.', '_', ':', '/', '-', or '#'"
        ));
    }
    Ok(())
}

fn validate_text(field: &str, value: &str) -> Result<(), BenchmarkError> {
    if value.trim() != value || value.is_empty() || value.len() > 4_096 {
        return invalid(format!(
            "{field} must be non-empty, trimmed, and at most 4096 bytes"
        ));
    }
    if value.chars().any(char::is_control) {
        return invalid(format!("{field} must not contain control characters"));
    }
    Ok(())
}

fn validate_positive(field: &str, value: f64) -> Result<(), BenchmarkError> {
    if !value.is_finite() || value <= 0.0 {
        return invalid(format!("{field} must be finite and strictly positive"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, BenchmarkError> {
    Err(BenchmarkError::InvalidProfile(message.into()))
}
