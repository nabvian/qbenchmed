//! Deterministic structural measurements for benchmark profiles.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{BenchmarkError, BenchmarkProfile};

/// Structural facts about one validated profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralMetrics {
    /// Number of selectable inputs.
    pub input_count: usize,
    /// Number of declared outcomes.
    pub outcome_count: usize,
    /// Number of binary incidence relationships.
    pub relationship_count: usize,
    /// Relationship count divided by all possible input/outcome pairs.
    pub incidence_density: f64,
    /// Sum of all input costs.
    pub total_input_cost: f64,
    /// Sum of all outcome weights.
    pub total_outcome_weight: f64,
    /// Sum of the weights of outcomes reachable by at least one input.
    pub reachable_outcome_weight: f64,
    /// Sorted inputs incident to at least one outcome.
    pub active_inputs: Vec<String>,
    /// Sorted inputs incident to no outcome.
    pub inert_inputs: Vec<String>,
    /// Sorted outcomes incident to at least one input.
    pub reachable_outcomes: Vec<String>,
    /// Sorted outcomes incident to no input.
    pub unreachable_outcomes: Vec<String>,
    /// Largest number of outcomes covered by one input.
    pub maximum_input_degree: usize,
    /// Largest number of inputs capable of covering one outcome.
    pub maximum_outcome_degree: usize,
}

/// Compute structural facts without solving an optimization problem.
#[allow(clippy::cast_precision_loss)] // Ratios intentionally expose potentially large counts as f64.
pub fn structural_metrics(profile: &BenchmarkProfile) -> Result<StructuralMetrics, BenchmarkError> {
    profile.validate()?;

    let mut input_degree: BTreeMap<&str, usize> = profile
        .inputs
        .iter()
        .map(|input| (input.id.as_str(), 0))
        .collect();
    let mut outcome_degree: BTreeMap<&str, usize> = profile
        .outcomes
        .iter()
        .map(|outcome| (outcome.id.as_str(), 0))
        .collect();
    for relationship in &profile.relationships {
        let input = input_degree
            .get_mut(relationship.input_id.as_str())
            .ok_or_else(|| {
                BenchmarkError::InvalidProfile(format!(
                    "relationship references unknown input {:?}",
                    relationship.input_id
                ))
            })?;
        *input += 1;
        let outcome = outcome_degree
            .get_mut(relationship.outcome_id.as_str())
            .ok_or_else(|| {
                BenchmarkError::InvalidProfile(format!(
                    "relationship references unknown outcome {:?}",
                    relationship.outcome_id
                ))
            })?;
        *outcome += 1;
    }

    let active_inputs: Vec<_> = input_degree
        .iter()
        .filter(|(_, degree)| **degree > 0)
        .map(|(id, _)| (*id).to_owned())
        .collect();
    let inert_inputs: Vec<_> = input_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| (*id).to_owned())
        .collect();
    let reachable_set: BTreeSet<_> = outcome_degree
        .iter()
        .filter(|(_, degree)| **degree > 0)
        .map(|(id, _)| *id)
        .collect();
    let reachable_outcomes = reachable_set.iter().map(|id| (*id).to_owned()).collect();
    let unreachable_outcomes = outcome_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| (*id).to_owned())
        .collect();

    let possible = profile.inputs.len() * profile.outcomes.len();
    let incidence_density = profile.relationships.len() as f64 / possible as f64;
    let reachable_outcome_weight = profile
        .outcomes
        .iter()
        .filter(|outcome| reachable_set.contains(outcome.id.as_str()))
        .map(|outcome| outcome.weight)
        .sum();

    Ok(StructuralMetrics {
        input_count: profile.inputs.len(),
        outcome_count: profile.outcomes.len(),
        relationship_count: profile.relationships.len(),
        incidence_density,
        total_input_cost: profile.inputs.iter().map(|input| input.cost).sum(),
        total_outcome_weight: profile.outcomes.iter().map(|outcome| outcome.weight).sum(),
        reachable_outcome_weight,
        active_inputs,
        inert_inputs,
        reachable_outcomes,
        unreachable_outcomes,
        maximum_input_degree: input_degree.values().copied().max().unwrap_or(0),
        maximum_outcome_degree: outcome_degree.values().copied().max().unwrap_or(0),
    })
}
