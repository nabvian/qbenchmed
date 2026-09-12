//! Version-to-version benchmark-profile comparison.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    BenchmarkError, BenchmarkProfile, IncidenceRelationship, PROFILE_DIFF_SCHEMA_VERSION,
    StructuralMetrics, structural_metrics,
};

/// Deterministic semantic delta between two versions of one benchmark profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDiff {
    /// Output schema identifier.
    pub schema_version: String,
    /// Stable profile identity shared by both versions.
    pub profile_id: String,
    /// Input identities present only in the new profile.
    pub inputs_added: Vec<String>,
    /// Input identities present only in the old profile.
    pub inputs_removed: Vec<String>,
    /// Shared input identities whose label, cost, or tags changed.
    pub inputs_modified: Vec<String>,
    /// Outcome identities present only in the new profile.
    pub outcomes_added: Vec<String>,
    /// Outcome identities present only in the old profile.
    pub outcomes_removed: Vec<String>,
    /// Shared outcome identities whose label, weight, or tags changed.
    pub outcomes_modified: Vec<String>,
    /// Incidences present only in the new profile.
    pub relationships_added: Vec<IncidenceRelationship>,
    /// Incidences present only in the old profile.
    pub relationships_removed: Vec<IncidenceRelationship>,
    /// Outcomes now unreachable that were not previously unreachable.
    pub newly_unreachable_outcomes: Vec<String>,
    /// Outcomes now reachable that were previously unreachable.
    pub newly_reachable_outcomes: Vec<String>,
    /// Previously inert inputs that now cover at least one outcome.
    pub newly_active_inputs: Vec<String>,
    /// Previously active inputs that now cover no outcome.
    pub newly_inert_inputs: Vec<String>,
    /// Whether declared feasibility constraints changed.
    pub constraints_changed: bool,
    /// Whether the primary objective changed.
    pub objective_changed: bool,
    /// Structural measurements before the change.
    pub before: StructuralMetrics,
    /// Structural measurements after the change.
    pub after: StructuralMetrics,
}

/// Compare two validated versions that share the same stable profile identity.
pub fn diff_profiles(
    old: &BenchmarkProfile,
    new: &BenchmarkProfile,
) -> Result<ProfileDiff, BenchmarkError> {
    old.validate()?;
    new.validate()?;
    if old.profile_id != new.profile_id {
        return Err(BenchmarkError::InvalidRequest(format!(
            "cannot diff profile {:?} against {:?}",
            old.profile_id, new.profile_id
        )));
    }

    let old_inputs: BTreeMap<_, _> = old
        .inputs
        .iter()
        .map(|input| (input.id.as_str(), input))
        .collect();
    let new_inputs: BTreeMap<_, _> = new
        .inputs
        .iter()
        .map(|input| (input.id.as_str(), input))
        .collect();
    let old_outcomes: BTreeMap<_, _> = old
        .outcomes
        .iter()
        .map(|outcome| (outcome.id.as_str(), outcome))
        .collect();
    let new_outcomes: BTreeMap<_, _> = new
        .outcomes
        .iter()
        .map(|outcome| (outcome.id.as_str(), outcome))
        .collect();

    let before = structural_metrics(old)?;
    let after = structural_metrics(new)?;
    let old_rel: BTreeSet<_> = old.relationships.iter().cloned().collect();
    let new_rel: BTreeSet<_> = new.relationships.iter().cloned().collect();

    Ok(ProfileDiff {
        schema_version: PROFILE_DIFF_SCHEMA_VERSION.to_owned(),
        profile_id: old.profile_id.clone(),
        inputs_added: key_difference(&new_inputs, &old_inputs),
        inputs_removed: key_difference(&old_inputs, &new_inputs),
        inputs_modified: modified_keys(&old_inputs, &new_inputs),
        outcomes_added: key_difference(&new_outcomes, &old_outcomes),
        outcomes_removed: key_difference(&old_outcomes, &new_outcomes),
        outcomes_modified: modified_keys(&old_outcomes, &new_outcomes),
        relationships_added: new_rel.difference(&old_rel).cloned().collect(),
        relationships_removed: old_rel.difference(&new_rel).cloned().collect(),
        newly_unreachable_outcomes: difference(
            &after.unreachable_outcomes,
            &before.unreachable_outcomes,
        ),
        newly_reachable_outcomes: intersection(
            &before.unreachable_outcomes,
            &after.reachable_outcomes,
        ),
        newly_active_inputs: intersection(&before.inert_inputs, &after.active_inputs),
        newly_inert_inputs: intersection(&before.active_inputs, &after.inert_inputs),
        constraints_changed: old.constraints != new.constraints,
        objective_changed: old.objective != new.objective,
        before,
        after,
    })
}

fn key_difference<V>(left: &BTreeMap<&str, V>, right: &BTreeMap<&str, V>) -> Vec<String> {
    left.keys()
        .filter(|key| !right.contains_key(**key))
        .map(|key| (*key).to_owned())
        .collect()
}

fn modified_keys<V: PartialEq>(old: &BTreeMap<&str, V>, new: &BTreeMap<&str, V>) -> Vec<String> {
    old.iter()
        .filter_map(|(key, old_value)| {
            new.get(key)
                .filter(|new_value| *new_value != old_value)
                .map(|_| (*key).to_owned())
        })
        .collect()
}

fn difference(left: &[String], right: &[String]) -> Vec<String> {
    let right: BTreeSet<_> = right.iter().map(String::as_str).collect();
    left.iter()
        .filter(|value| !right.contains(value.as_str()))
        .cloned()
        .collect()
}

fn intersection(left: &[String], right: &[String]) -> Vec<String> {
    let right: BTreeSet<_> = right.iter().map(String::as_str).collect();
    left.iter()
        .filter(|value| right.contains(value.as_str()))
        .cloned()
        .collect()
}
