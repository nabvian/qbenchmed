use std::collections::BTreeSet;

use crate::{
    PROJECTION_CANDIDATE_SCHEMA_VERSION, ProfilerError, ProjectionCandidate, ProjectionEvidence,
};

pub(crate) fn validate_candidate(candidate: &ProjectionCandidate) -> Result<(), ProfilerError> {
    if candidate.schema_version != PROJECTION_CANDIDATE_SCHEMA_VERSION {
        return invalid(format!(
            "unsupported schema_version {:?}; expected {PROJECTION_CANDIDATE_SCHEMA_VERSION}",
            candidate.schema_version
        ));
    }
    if candidate.confidence_bps > 10_000 {
        return invalid("confidence_bps must be at most 10000");
    }
    candidate
        .profile
        .validate()
        .map_err(ProfilerError::InvalidProfile)?;

    ensure_sorted_unique("extracted_fields", &candidate.extracted_fields)?;
    ensure_sorted_unique("defaulted_fields", &candidate.defaulted_fields)?;
    if let Some(overlap) = candidate
        .extracted_fields
        .iter()
        .collect::<BTreeSet<_>>()
        .intersection(&candidate.defaulted_fields.iter().collect())
        .next()
    {
        return invalid(format!(
            "field {overlap:?} cannot be both extracted and defaulted"
        ));
    }
    ensure_strictly_sorted("evidence", &candidate.evidence)?;
    ensure_strictly_sorted("diagnostics", &candidate.diagnostics)?;
    ensure_strictly_sorted("assumptions", &candidate.assumptions)?;
    if candidate.evidence.is_empty() {
        return invalid("evidence must not be empty");
    }

    let global_evidence: BTreeSet<_> = candidate.evidence.iter().collect();
    for diagnostic in &candidate.diagnostics {
        ensure_strictly_sorted("diagnostic.evidence", &diagnostic.evidence)?;
        ensure_nested_evidence(&global_evidence, &diagnostic.evidence)?;
    }
    for assumption in &candidate.assumptions {
        ensure_strictly_sorted("assumption.evidence", &assumption.evidence)?;
        ensure_nested_evidence(&global_evidence, &assumption.evidence)?;
    }

    for field in candidate
        .defaulted_fields
        .iter()
        .filter(|field| field.ends_with("/cost") || field.ends_with("/weight"))
    {
        if !candidate
            .assumptions
            .iter()
            .any(|assumption| assumption.target_field == *field && assumption.value == "1")
        {
            return invalid(format!(
                "defaulted numeric field {field:?} must have an explicit value-1 assumption"
            ));
        }
    }
    Ok(())
}

fn ensure_nested_evidence(
    global: &BTreeSet<&ProjectionEvidence>,
    nested: &[ProjectionEvidence],
) -> Result<(), ProfilerError> {
    if let Some(missing) = nested.iter().find(|item| !global.contains(item)) {
        return invalid(format!(
            "nested evidence at {:?} is absent from candidate evidence",
            missing.pointer
        ));
    }
    Ok(())
}

fn ensure_sorted_unique(field: &str, values: &[String]) -> Result<(), ProfilerError> {
    if values.iter().any(String::is_empty) {
        return invalid(format!("{field} must not contain empty values"));
    }
    ensure_strictly_sorted(field, values)
}

fn ensure_strictly_sorted<T: Ord>(field: &str, values: &[T]) -> Result<(), ProfilerError> {
    if !values.windows(2).all(|pair| pair[0] < pair[1]) {
        return invalid(format!("{field} must be strictly sorted and unique"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ProfilerError> {
    Err(ProfilerError::InvalidCandidate(message.into()))
}
