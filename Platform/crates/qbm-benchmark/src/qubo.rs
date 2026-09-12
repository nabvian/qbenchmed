//! Exact QUBO/Ising encoding and transparent resource diagnostics.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    BenchmarkError, BenchmarkProfile, ISING_SCHEMA_VERSION, OptimizationRequest,
    QUBO_SCHEMA_VERSION, SelectionScore, evaluate_selection,
};

/// Semantic role of one QUBO binary variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuboVariableKind {
    /// Selects one biomedical input.
    Input,
    /// Marks one biomedical outcome as covered.
    Outcome,
    /// Binary slack bit used to encode an equality/inequality exactly.
    Slack,
    /// Marks one conjunctive arm of an outcome's rule as satisfied.
    ///
    /// An outcome covered by a combination of inputs cannot be expressed by a
    /// direct input-to-outcome term, so each arm gets a variable that may only
    /// rise when every input the arm needs is selected.
    Arm,
}

/// Indexed QUBO variable with an auditable semantic source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuboVariable {
    /// Zero-based position in assignments and linear coefficients.
    pub index: usize,
    /// Deterministic variable name.
    pub name: String,
    /// Semantic role.
    pub kind: QuboVariableKind,
    /// Referenced input/outcome/constraint identity.
    pub source_id: String,
}

/// One upper-triangular QUBO coupling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuboCoupling {
    /// Smaller variable index.
    pub left: usize,
    /// Larger variable index.
    pub right: usize,
    /// Coefficient multiplying `x_left * x_right`.
    pub coefficient: f64,
}

/// Deterministic QUBO representation of weighted maximum coverage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuboModel {
    /// Output schema identifier.
    pub schema_version: String,
    /// Stable source profile identity.
    pub profile_id: String,
    /// Request whose supported constraints were encoded.
    pub request: OptimizationRequest,
    /// Binary variables in assignment order.
    pub variables: Vec<QuboVariable>,
    /// Linear QUBO diagonal coefficients.
    pub linear: Vec<f64>,
    /// Sorted non-zero upper-triangular couplings.
    pub quadratic: Vec<QuboCoupling>,
    /// Constant energy offset.
    pub offset: f64,
    /// Constraint penalty, strictly larger than all attainable coverage reward.
    pub constraint_penalty: f64,
    /// Explicit, deterministic encoding limitations.
    pub encoding_notes: Vec<String>,
}

/// One upper-triangular Ising spin coupling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsingCoupling {
    /// Smaller spin index.
    pub left: usize,
    /// Larger spin index.
    pub right: usize,
    /// Coefficient multiplying `z_left * z_right` for spins in `{-1, +1}`.
    pub coefficient: f64,
}

/// Ising model exactly equivalent to a [`QuboModel`] under `x = (1-z)/2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsingModel {
    /// Output schema identifier.
    pub schema_version: String,
    /// Stable source profile identity.
    pub profile_id: String,
    /// Linear spin biases.
    pub linear: Vec<f64>,
    /// Sorted pairwise spin couplings.
    pub quadratic: Vec<IsingCoupling>,
    /// Constant energy offset.
    pub offset: f64,
}

/// Size and coefficient-range diagnostics for a QUBO/Ising instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuboMetrics {
    /// Total binary-variable/qubit count before hardware embedding.
    pub variable_count: usize,
    /// Variables representing biomedical inputs.
    pub input_variable_count: usize,
    /// Variables representing biomedical outcomes.
    pub outcome_variable_count: usize,
    /// Ancillary constraint slack variables.
    pub slack_variable_count: usize,
    /// Variables representing one conjunctive arm of an outcome's rule.
    #[serde(default)]
    pub arm_variable_count: usize,
    /// Number of non-zero linear coefficients.
    pub nonzero_linear_count: usize,
    /// Number of non-zero pair couplings.
    pub coupling_count: usize,
    /// Coupling count divided by all possible variable pairs.
    pub coupling_density: f64,
    /// Largest logical coupling degree.
    pub maximum_degree: usize,
    /// Smallest absolute non-zero linear/coupling coefficient.
    pub minimum_absolute_coefficient: f64,
    /// Largest absolute linear/coupling coefficient.
    pub maximum_absolute_coefficient: f64,
    /// Maximum divided by minimum absolute coefficient.
    pub coefficient_dynamic_range: f64,
    /// Constraint penalty chosen by the encoder.
    pub constraint_penalty: f64,
}

/// Qualitative bin for a transparent, non-performance difficulty score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DifficultyLevel {
    /// Small/simple by the documented structural heuristic.
    Low,
    /// Some size, density, or scaling pressure.
    Moderate,
    /// Material logical resource or coefficient-scaling pressure.
    High,
    /// Very large or ill-scaled relative to local exact baselines.
    Extreme,
}

/// One explained contribution to a difficulty estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DifficultyFactor {
    /// Stable factor identity.
    pub name: String,
    /// Integer points contributed to the 0..=100 score.
    pub contribution: u8,
    /// Human-readable measured reason.
    pub observation: String,
}

/// Transparent structural estimate; not a runtime or quantum-advantage claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DifficultyEstimate {
    /// Saturating structural score in `0..=100`.
    pub score: u8,
    /// Qualitative interpretation of `score`.
    pub level: DifficultyLevel,
    /// Deterministically ordered contributions.
    pub factors: Vec<DifficultyFactor>,
    /// Mandatory interpretation boundary.
    pub disclaimer: String,
}

/// Construct an exact binary QUBO for the supported maximum-coverage model.
///
/// The encoding uses one outcome variable plus binary slack for each incidence
/// equality, and optional slack for panel-size bounds. Floating-point cost
/// budgets and coverage-floor constraints are rejected rather than omitted.
#[allow(clippy::cast_precision_loss)] // Integer constraint weights are represented in f64 QUBO coefficients.
#[allow(clippy::too_many_lines)] // Keeping the complete encoding together makes its constraints auditable.
pub fn build_qubo(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
) -> Result<QuboModel, BenchmarkError> {
    profile.validate()?;
    if profile.constraints.max_total_cost.is_some() {
        return Err(BenchmarkError::UnsupportedQuboConstraint(
            "floating-point max_total_cost; use an explicitly reviewed integer scaling".to_owned(),
        ));
    }
    if request.coverage_floor.is_some() {
        return Err(BenchmarkError::UnsupportedQuboConstraint(
            "coverage_floor; construct maximum coverage first and rescore natively".to_owned(),
        ));
    }
    if let Some(maximum) = request.max_inputs {
        if maximum == 0 || maximum > profile.inputs.len() {
            return Err(BenchmarkError::InvalidRequest(
                "max_inputs must be positive and not exceed the profile input count".to_owned(),
            ));
        }
        if maximum < profile.constraints.min_selected
            || maximum < profile.constraints.required_inputs.len()
        {
            return Err(BenchmarkError::Infeasible(
                "max_inputs is below a profile-level lower bound".to_owned(),
            ));
        }
    }

    let total_weight: f64 = profile.outcomes.iter().map(|outcome| outcome.weight).sum();
    let penalty = total_weight + 1.0;
    if !penalty.is_finite() {
        return Err(BenchmarkError::NonFinite("QUBO constraint penalty"));
    }

    let mut builder = QuboBuilder::default();
    let mut input_indices = BTreeMap::new();
    for input in &profile.inputs {
        let index = builder.variable(
            format!("input:{}", input.id),
            QuboVariableKind::Input,
            input.id.clone(),
        );
        input_indices.insert(input.id.as_str(), index);
    }
    let mut outcome_indices = BTreeMap::new();
    for outcome in &profile.outcomes {
        let index = builder.variable(
            format!("outcome:{}", outcome.id),
            QuboVariableKind::Outcome,
            outcome.id.clone(),
        );
        outcome_indices.insert(outcome.id.as_str(), index);
        builder.add_linear(index, -outcome.weight);
    }

    // Coverage is decided arm by arm, so the encoding needs one variable per
    // arm sitting between the inputs and the outcome. Reading the relationship
    // list directly would encode "any one input covers the outcome", which is
    // the wrong problem for every profile whose triggers fire on combinations.
    let model = crate::coverage::CoverageModel::compile(profile)?;
    for (position, outcome) in profile.outcomes.iter().enumerate() {
        let outcome_index = outcome_indices[outcome.id.as_str()];
        let rule = &model.rules[position];

        if rule.unconditional {
            // Covered by every selection, so nothing constrains the variable
            // and the reward already on it stands.
            continue;
        }
        if rule.arms.is_empty() {
            builder.add_linear(outcome_index, penalty);
            continue;
        }

        let mut arm_indices = Vec::with_capacity(rule.arms.len());
        for (ordinal, arm) in rule.arms.iter().enumerate() {
            let label = if arm.path.is_empty() {
                format!("{ordinal}")
            } else {
                arm.path.clone()
            };
            let arm_index = builder.variable(
                format!("arm:{}:{label}", outcome.id),
                QuboVariableKind::Arm,
                format!("arm:{}:{label}", outcome.id),
            );
            arm_indices.push(arm_index);

            // An arm may only fire when every input it needs is selected and
            // every input it forbids is not. Both directions are one-sided:
            // nothing rewards an arm directly, so the objective can only push
            // it up, and these terms are what stop it rising for free.
            //
            //   u * (1 - x) penalises firing without a needed input
            //   u * x       penalises firing despite a forbidden one
            arm.present_closure.for_each(|input| {
                let x = input_indices[profile.inputs[input].id.as_str()];
                builder.add_linear(arm_index, penalty);
                builder.add_quadratic(arm_index, x, -penalty);
            });
            arm.absent_closure.for_each(|input| {
                let x = input_indices[profile.inputs[input].id.as_str()];
                builder.add_quadratic(arm_index, x, penalty);
            });
        }

        // The outcome fires when at least one of its arms does.
        let mut expression: Vec<_> = arm_indices.iter().map(|index| (*index, 1.0)).collect();
        expression.push((outcome_index, -1.0));
        for (bit, weight) in binary_weights(arm_indices.len()).into_iter().enumerate() {
            let index = builder.variable(
                format!("slack:outcome:{}:{bit}", outcome.id),
                QuboVariableKind::Slack,
                format!("coverage:{}", outcome.id),
            );
            expression.push((index, -(weight as f64)));
        }
        builder.add_square(&expression, 0.0, penalty);
    }

    let effective_maximum = [
        Some(profile.inputs.len()),
        profile.constraints.max_selected,
        request.max_inputs,
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(profile.inputs.len());
    if effective_maximum < profile.inputs.len() {
        let mut expression: Vec<_> = input_indices.values().map(|index| (*index, 1.0)).collect();
        for (bit, weight) in binary_weights(effective_maximum).into_iter().enumerate() {
            let index = builder.variable(
                format!("slack:max_selected:{bit}"),
                QuboVariableKind::Slack,
                "max_selected".to_owned(),
            );
            expression.push((index, weight as f64));
        }
        builder.add_square(&expression, -(effective_maximum as f64), penalty);
    }

    if profile.constraints.min_selected > 0 {
        let mut expression: Vec<_> = input_indices.values().map(|index| (*index, 1.0)).collect();
        let maximum_slack = profile.inputs.len() - profile.constraints.min_selected;
        for (bit, weight) in binary_weights(maximum_slack).into_iter().enumerate() {
            let index = builder.variable(
                format!("slack:min_selected:{bit}"),
                QuboVariableKind::Slack,
                "min_selected".to_owned(),
            );
            expression.push((index, -(weight as f64)));
        }
        builder.add_square(
            &expression,
            -(profile.constraints.min_selected as f64),
            penalty,
        );
    }

    for required in &profile.constraints.required_inputs {
        builder.add_square(&[(input_indices[required.as_str()], -1.0)], 1.0, penalty);
    }
    for excluded in &profile.constraints.excluded_inputs {
        builder.add_square(&[(input_indices[excluded.as_str()], 1.0)], 0.0, penalty);
    }
    for required in &profile.constraints.required_outcomes {
        builder.add_square(&[(outcome_indices[required.as_str()], -1.0)], 1.0, penalty);
    }

    builder.finish(profile, request, penalty)
}

impl QuboModel {
    /// Evaluate `offset + linear + quadratic` for a full binary assignment.
    pub fn energy(&self, assignment: &[bool]) -> Result<f64, BenchmarkError> {
        if assignment.len() != self.variables.len() {
            return Err(BenchmarkError::InvalidAssignment(format!(
                "expected {} binary values, received {}",
                self.variables.len(),
                assignment.len()
            )));
        }
        let mut energy = self.offset;
        for (index, coefficient) in self.linear.iter().enumerate() {
            if assignment[index] {
                energy += coefficient;
            }
        }
        for coupling in &self.quadratic {
            if assignment[coupling.left] && assignment[coupling.right] {
                energy += coupling.coefficient;
            }
        }
        if !energy.is_finite() {
            return Err(BenchmarkError::NonFinite("QUBO energy"));
        }
        Ok(energy)
    }

    /// Rescore the input-variable portion of an assignment in native profile units.
    pub fn native_rescore(
        &self,
        profile: &BenchmarkProfile,
        assignment: &[bool],
    ) -> Result<SelectionScore, BenchmarkError> {
        if profile.profile_id != self.profile_id {
            return Err(BenchmarkError::InvalidRequest(format!(
                "QUBO profile {:?} does not match {:?}",
                self.profile_id, profile.profile_id
            )));
        }
        if assignment.len() != self.variables.len() {
            return Err(BenchmarkError::InvalidAssignment(format!(
                "expected {} binary values, received {}",
                self.variables.len(),
                assignment.len()
            )));
        }
        let selected: Vec<_> = self
            .variables
            .iter()
            .filter(|variable| {
                variable.kind == QuboVariableKind::Input && assignment[variable.index]
            })
            .map(|variable| variable.source_id.clone())
            .collect();
        evaluate_selection(profile, &selected)
    }

    /// Convert this model exactly to Ising form under `x = (1-z)/2`.
    pub fn to_ising(&self) -> Result<IsingModel, BenchmarkError> {
        let mut linear: Vec<_> = self
            .linear
            .iter()
            .map(|coefficient| -coefficient / 2.0)
            .collect();
        let mut quadratic = Vec::with_capacity(self.quadratic.len());
        let mut offset = self.offset + self.linear.iter().sum::<f64>() / 2.0;
        for coupling in &self.quadratic {
            let coefficient = coupling.coefficient / 4.0;
            quadratic.push(IsingCoupling {
                left: coupling.left,
                right: coupling.right,
                coefficient,
            });
            linear[coupling.left] -= coupling.coefficient / 4.0;
            linear[coupling.right] -= coupling.coefficient / 4.0;
            offset += coupling.coefficient / 4.0;
        }
        if !offset.is_finite()
            || linear.iter().any(|coefficient| !coefficient.is_finite())
            || quadratic
                .iter()
                .any(|coupling| !coupling.coefficient.is_finite())
        {
            return Err(BenchmarkError::NonFinite("Ising coefficients"));
        }
        Ok(IsingModel {
            schema_version: ISING_SCHEMA_VERSION.to_owned(),
            profile_id: self.profile_id.clone(),
            linear,
            quadratic,
            offset,
        })
    }

    /// Compute logical size, density, and coefficient dynamic range.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // Density is defined as a floating-point ratio of counts.
    pub fn metrics(&self) -> QuboMetrics {
        let variable_count = self.variables.len();
        let mut degree = vec![0_usize; variable_count];
        for coupling in &self.quadratic {
            degree[coupling.left] += 1;
            degree[coupling.right] += 1;
        }
        let coefficients: Vec<_> = self
            .linear
            .iter()
            .copied()
            .chain(self.quadratic.iter().map(|coupling| coupling.coefficient))
            .map(f64::abs)
            .filter(|coefficient| *coefficient > 0.0)
            .collect();
        let minimum = coefficients
            .iter()
            .copied()
            .min_by(f64::total_cmp)
            .unwrap_or(0.0);
        let maximum = coefficients
            .iter()
            .copied()
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        let possible_couplings =
            variable_count.saturating_mul(variable_count.saturating_sub(1)) / 2;
        QuboMetrics {
            variable_count,
            input_variable_count: self
                .variables
                .iter()
                .filter(|variable| variable.kind == QuboVariableKind::Input)
                .count(),
            outcome_variable_count: self
                .variables
                .iter()
                .filter(|variable| variable.kind == QuboVariableKind::Outcome)
                .count(),
            slack_variable_count: self
                .variables
                .iter()
                .filter(|variable| variable.kind == QuboVariableKind::Slack)
                .count(),
            arm_variable_count: self
                .variables
                .iter()
                .filter(|variable| variable.kind == QuboVariableKind::Arm)
                .count(),
            nonzero_linear_count: self.linear.iter().filter(|value| **value != 0.0).count(),
            coupling_count: self.quadratic.len(),
            coupling_density: if possible_couplings == 0 {
                0.0
            } else {
                self.quadratic.len() as f64 / possible_couplings as f64
            },
            maximum_degree: degree.into_iter().max().unwrap_or(0),
            minimum_absolute_coefficient: minimum,
            maximum_absolute_coefficient: maximum,
            coefficient_dynamic_range: if minimum == 0.0 {
                0.0
            } else {
                maximum / minimum
            },
            constraint_penalty: self.constraint_penalty,
        }
    }
}

impl IsingModel {
    /// Evaluate Ising energy for a full assignment of spins in `{-1, +1}`.
    pub fn energy(&self, spins: &[i8]) -> Result<f64, BenchmarkError> {
        if spins.len() != self.linear.len() || spins.iter().any(|spin| !matches!(spin, -1 | 1)) {
            return Err(BenchmarkError::InvalidAssignment(format!(
                "expected {} spins, each -1 or +1",
                self.linear.len()
            )));
        }
        let mut energy = self.offset;
        for (coefficient, spin) in self.linear.iter().zip(spins) {
            energy += coefficient * f64::from(*spin);
        }
        for coupling in &self.quadratic {
            energy += coupling.coefficient
                * f64::from(spins[coupling.left])
                * f64::from(spins[coupling.right]);
        }
        if !energy.is_finite() {
            return Err(BenchmarkError::NonFinite("Ising energy"));
        }
        Ok(energy)
    }
}

/// Estimate structural difficulty from size, density, degree, and dynamic range.
///
/// This intentionally does not predict wall-clock runtime, solution quality, or
/// quantum advantage.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)] // Every score conversion is rounded and clamped to its documented u8 range.
pub fn estimate_difficulty(
    profile: &BenchmarkProfile,
    metrics: &QuboMetrics,
) -> DifficultyEstimate {
    let size_points: u8 = match metrics.variable_count {
        0..=32 => 2,
        33..=128 => 10,
        129..=512 => 22,
        513..=2_048 => 32,
        _ => 40,
    };
    let density_points = (metrics.coupling_density * 20.0).round().clamp(0.0, 20.0) as u8;
    let range_points = if metrics.coefficient_dynamic_range <= 1.0 {
        0
    } else {
        (metrics.coefficient_dynamic_range.log10() * 6.0)
            .round()
            .clamp(0.0, 20.0) as u8
    };
    let slack_ratio = if metrics.variable_count == 0 {
        0.0
    } else {
        metrics.slack_variable_count as f64 / metrics.variable_count as f64
    };
    let slack_points = (slack_ratio * 10.0).round().clamp(0.0, 10.0) as u8;
    let degree_ratio = if metrics.variable_count <= 1 {
        0.0
    } else {
        metrics.maximum_degree as f64 / (metrics.variable_count - 1) as f64
    };
    let degree_points = (degree_ratio * 10.0).round().clamp(0.0, 10.0) as u8;
    let exact_points = match profile.inputs.len() {
        0..=16 => 0,
        17..=20 => 2,
        21..=30 => 5,
        _ => 10,
    };
    let score = size_points
        .saturating_add(density_points)
        .saturating_add(range_points)
        .saturating_add(slack_points)
        .saturating_add(degree_points)
        .saturating_add(exact_points)
        .min(100);
    let level = match score {
        0..=24 => DifficultyLevel::Low,
        25..=49 => DifficultyLevel::Moderate,
        50..=74 => DifficultyLevel::High,
        _ => DifficultyLevel::Extreme,
    };
    DifficultyEstimate {
        score,
        level,
        factors: vec![
            DifficultyFactor {
                name: "logical_size".to_owned(),
                contribution: size_points,
                observation: format!("{} binary variables", metrics.variable_count),
            },
            DifficultyFactor {
                name: "coupling_density".to_owned(),
                contribution: density_points,
                observation: format!("{:.6} logical density", metrics.coupling_density),
            },
            DifficultyFactor {
                name: "coefficient_dynamic_range".to_owned(),
                contribution: range_points,
                observation: format!("{:.6} max/min coefficient ratio", metrics.coefficient_dynamic_range),
            },
            DifficultyFactor {
                name: "ancillary_slack".to_owned(),
                contribution: slack_points,
                observation: format!("{} slack variables", metrics.slack_variable_count),
            },
            DifficultyFactor {
                name: "maximum_degree".to_owned(),
                contribution: degree_points,
                observation: format!("maximum logical degree {}", metrics.maximum_degree),
            },
            DifficultyFactor {
                name: "classical_exact_scale".to_owned(),
                contribution: exact_points,
                observation: format!("{} native input variables", profile.inputs.len()),
            },
        ],
        disclaimer: "structural estimate only; it is not a runtime prediction, hardware-embedding estimate, or quantum-advantage claim".to_owned(),
    }
}

#[derive(Debug, Default)]
struct QuboBuilder {
    variables: Vec<QuboVariable>,
    linear: Vec<f64>,
    quadratic: BTreeMap<(usize, usize), f64>,
    offset: f64,
}

impl QuboBuilder {
    fn variable(&mut self, name: String, kind: QuboVariableKind, source_id: String) -> usize {
        let index = self.variables.len();
        self.variables.push(QuboVariable {
            index,
            name,
            kind,
            source_id,
        });
        self.linear.push(0.0);
        index
    }

    fn add_linear(&mut self, index: usize, coefficient: f64) {
        self.linear[index] += coefficient;
    }

    fn add_quadratic(&mut self, left: usize, right: usize, coefficient: f64) {
        let key = if left < right {
            (left, right)
        } else {
            (right, left)
        };
        *self.quadratic.entry(key).or_default() += coefficient;
    }

    fn add_square(&mut self, terms: &[(usize, f64)], constant: f64, penalty: f64) {
        let mut combined: BTreeMap<usize, f64> = BTreeMap::new();
        for (index, coefficient) in terms {
            *combined.entry(*index).or_default() += coefficient;
        }
        let terms: Vec<_> = combined.into_iter().collect();
        self.offset += penalty * constant * constant;
        for (position, (index, coefficient)) in terms.iter().enumerate() {
            self.linear[*index] +=
                penalty * (coefficient * coefficient + 2.0 * constant * coefficient);
            for (other_index, other_coefficient) in terms.iter().skip(position + 1) {
                let key = if index < other_index {
                    (*index, *other_index)
                } else {
                    (*other_index, *index)
                };
                *self.quadratic.entry(key).or_default() +=
                    2.0 * penalty * coefficient * other_coefficient;
            }
        }
    }

    fn finish(
        self,
        profile: &BenchmarkProfile,
        request: &OptimizationRequest,
        penalty: f64,
    ) -> Result<QuboModel, BenchmarkError> {
        if !self.offset.is_finite()
            || self.linear.iter().any(|value| !value.is_finite())
            || self.quadratic.values().any(|value| !value.is_finite())
        {
            return Err(BenchmarkError::NonFinite("QUBO coefficients"));
        }
        Ok(QuboModel {
            schema_version: QUBO_SCHEMA_VERSION.to_owned(),
            profile_id: profile.profile_id.clone(),
            request: request.clone(),
            variables: self.variables,
            linear: self.linear,
            quadratic: self
                .quadratic
                .into_iter()
                .filter(|(_, coefficient)| *coefficient != 0.0)
                .map(|((left, right), coefficient)| QuboCoupling {
                    left,
                    right,
                    coefficient,
                })
                .collect(),
            offset: self.offset,
            constraint_penalty: penalty,
            encoding_notes: vec![
                "logical QUBO only; no hardware topology embedding is included".to_owned(),
                "input costs are native rescore/tie-break data, not part of the primary QUBO reward"
                    .to_owned(),
                "quantum execution and quantum advantage are not claimed".to_owned(),
            ],
        })
    }
}

fn binary_weights(maximum: usize) -> Vec<usize> {
    if maximum == 0 {
        return Vec::new();
    }
    let bits = usize::BITS as usize - maximum.leading_zeros() as usize;
    (0..bits).map(|bit| 1_usize << bit).collect()
}
