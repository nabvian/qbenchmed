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
    /// Sorted inputs that can advance at least one outcome's rule.
    pub active_inputs: Vec<String>,
    /// Sorted inputs no relationship mentions at all.
    pub inert_inputs: Vec<String>,
    /// Sorted inputs that appear only as a block or as another edge's context,
    /// so selecting one can never grant coverage.
    ///
    /// Distinct from inert: the profile does wire these, but only ever to say
    /// "not this". Folding them in with the unmentioned inputs would hide the
    /// more interesting of the two cases.
    #[serde(default)]
    pub veto_only_inputs: Vec<String>,
    /// Sorted outcomes some selection can cover.
    pub reachable_outcomes: Vec<String>,
    /// Sorted outcomes no selection can cover.
    ///
    /// Under typed semantics an outcome can be unreachable even though
    /// relationships mention it, when its arms can never be satisfied
    /// together. That is a finding about the rule set, not a bookkeeping
    /// artefact.
    pub unreachable_outcomes: Vec<String>,
    /// Sorted outcomes covered by every selection, including the empty one.
    #[serde(default)]
    pub unconditional_outcomes: Vec<String>,
    /// Number of conjunctive arms across all outcomes.
    #[serde(default)]
    pub arm_count: usize,
    /// Number of arms needing more than one input present at once.
    #[serde(default)]
    pub conjunctive_arm_count: usize,
    /// Whether coverage reduces to plain binary incidence.
    ///
    /// When false, coverage is not submodular and greedy carries no
    /// approximation guarantee on this profile.
    #[serde(default)]
    pub purely_disjunctive: bool,
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

    // Roles and reachability come from the compiled rules, not from raw
    // incidence counts. An input can appear in a dozen rows and still never be
    // able to cover anything, and an outcome can have rows yet be unreachable
    // because its arms contradict each other.
    let model = crate::coverage::CoverageModel::compile(profile)?;
    let roles = model.contribution_roles();
    let mut active_inputs = Vec::new();
    let mut inert_inputs = Vec::new();
    let mut veto_only_inputs = Vec::new();
    for (index, input) in profile.inputs.iter().enumerate() {
        match roles[index] {
            crate::coverage::InputRole::Contributing => active_inputs.push(input.id.clone()),
            crate::coverage::InputRole::VetoOnly => veto_only_inputs.push(input.id.clone()),
            crate::coverage::InputRole::Inert => inert_inputs.push(input.id.clone()),
        }
    }

    let mut reachable_set: BTreeSet<&str> = BTreeSet::new();
    let mut reachable_outcomes = Vec::new();
    let mut unreachable_outcomes = Vec::new();
    for (index, outcome) in profile.outcomes.iter().enumerate() {
        if model.is_reachable(index) {
            reachable_set.insert(outcome.id.as_str());
            reachable_outcomes.push(outcome.id.clone());
        } else {
            unreachable_outcomes.push(outcome.id.clone());
        }
    }
    let arm_count: usize = model.rules.iter().map(|rule| rule.arms.len()).sum();
    let conjunctive_arm_count: usize = model
        .rules
        .iter()
        .flat_map(|rule| &rule.arms)
        .filter(|arm| arm.present_indices.len() > 1)
        .count();

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
        veto_only_inputs,
        reachable_outcomes,
        unreachable_outcomes,
        unconditional_outcomes: profile.unconditional_outcomes.clone(),
        arm_count,
        conjunctive_arm_count,
        purely_disjunctive: profile.is_purely_disjunctive(),
        maximum_input_degree: input_degree.values().copied().max().unwrap_or(0),
        maximum_outcome_degree: outcome_degree.values().copied().max().unwrap_or(0),
    })
}
