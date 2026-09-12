//! Deterministic, resource-bounded QUBO/Ising equivalence checks.

use qbm_benchmark::{IsingModel, QuboModel};
use serde::{Deserialize, Serialize};

use crate::{ENERGY_EQUIVALENCE_SCHEMA_VERSION, QuantumContractError, QuantumProblem};

/// Default maximum number of generated assignments checked per model pair.
pub const DEFAULT_ASSIGNMENT_LIMIT: usize = 4_096;
/// Hard ceiling accepted from callers for a single validation request.
pub const MAX_ASSIGNMENT_LIMIT: usize = 65_536;
const MAX_VARIABLE_EVALUATIONS: usize = 8_388_608;

/// Numeric tolerance and deterministic assignment ceiling for equivalence checks.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyEquivalenceConfig {
    /// Maximum generated assignments, constrained by [`MAX_ASSIGNMENT_LIMIT`].
    pub assignment_limit: usize,
    /// Fixed absolute tolerance applied to every energy comparison.
    pub absolute_tolerance: f64,
    /// Energy-scale-relative tolerance applied to every comparison.
    pub relative_tolerance: f64,
}

impl Default for EnergyEquivalenceConfig {
    fn default() -> Self {
        Self {
            assignment_limit: DEFAULT_ASSIGNMENT_LIMIT,
            absolute_tolerance: 1.0e-10,
            relative_tolerance: 1.0e-12,
        }
    }
}

impl EnergyEquivalenceConfig {
    fn validate(self) -> Result<(), QuantumContractError> {
        if self.assignment_limit == 0 || self.assignment_limit > MAX_ASSIGNMENT_LIMIT {
            return Err(QuantumContractError::InvalidContract(format!(
                "assignment_limit must be within 1..={MAX_ASSIGNMENT_LIMIT}"
            )));
        }
        if !self.absolute_tolerance.is_finite()
            || self.absolute_tolerance < 0.0
            || !self.relative_tolerance.is_finite()
            || self.relative_tolerance < 0.0
        {
            return Err(QuantumContractError::InvalidContract(
                "energy tolerances must be finite and non-negative".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Evidence that a bounded deterministic assignment set preserved energies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergyEquivalenceReport {
    /// Contract schema identifier.
    pub schema_version: String,
    /// Number of binary variables and Ising spins.
    pub variable_count: usize,
    /// Total assignment space when it fits in `u64`.
    pub assignment_space_size: Option<u64>,
    /// Caller-requested assignment ceiling.
    pub assignment_limit: usize,
    /// Deterministically generated assignments actually checked.
    pub assignments_checked: usize,
    /// Whether every assignment in the finite space was checked.
    pub exhaustive: bool,
    /// Largest absolute difference observed.
    pub maximum_absolute_error: f64,
    /// Absolute tolerance used by the check.
    pub absolute_tolerance: f64,
    /// Relative tolerance used by the check.
    pub relative_tolerance: f64,
    /// Fixed mapping used for every comparison.
    pub mapping: String,
    /// Explicit interpretation boundary.
    pub limitation: String,
}

/// Validate QUBO/Ising energy equivalence under `x = (1-z)/2`.
///
/// Small models are enumerated exhaustively. Larger models use the same
/// deterministic sequence on every platform: all-zero, all-one, alternating,
/// inverse-alternating, then fixed SplitMix-derived bit patterns. Both the
/// assignment count and aggregate variable evaluations are bounded.
pub fn validate_energy_equivalence(
    qubo: &QuboModel,
    ising: &IsingModel,
    config: EnergyEquivalenceConfig,
) -> Result<EnergyEquivalenceReport, QuantumContractError> {
    config.validate()?;
    QuantumProblem::Qubo(qubo.clone()).validate()?;
    QuantumProblem::Ising(ising.clone()).validate()?;
    if qubo.profile_id != ising.profile_id {
        return Err(QuantumContractError::InvalidContract(format!(
            "QUBO profile {:?} does not match Ising profile {:?}",
            qubo.profile_id, ising.profile_id
        )));
    }
    if qubo.variables.len() != ising.linear.len() {
        return Err(QuantumContractError::InvalidContract(format!(
            "QUBO has {} variables but Ising model has {} spins",
            qubo.variables.len(),
            ising.linear.len()
        )));
    }

    let variable_count = qubo.variables.len();
    let assignment_space_size = assignment_space_size(variable_count);
    let work_limit = MAX_VARIABLE_EVALUATIONS
        .checked_div(variable_count)
        .unwrap_or(config.assignment_limit)
        .max(1);
    let bounded_limit = config.assignment_limit.min(work_limit);
    let exhaustive = assignment_space_size
        .and_then(|size| usize::try_from(size).ok())
        .is_some_and(|size| size <= bounded_limit);
    let assignments_checked = if exhaustive {
        usize::try_from(assignment_space_size.unwrap_or(1)).unwrap_or(bounded_limit)
    } else {
        bounded_limit
    };

    let mut maximum_absolute_error = 0.0_f64;
    for assignment_index in 0..assignments_checked {
        let assignment = generated_assignment(assignment_index, variable_count, exhaustive);
        let spins: Vec<_> = assignment
            .iter()
            .map(|selected| if *selected { -1 } else { 1 })
            .collect();
        let qubo_energy = qubo.energy(&assignment)?;
        let ising_energy = ising.energy(&spins)?;
        let absolute_error = (qubo_energy - ising_energy).abs();
        maximum_absolute_error = maximum_absolute_error.max(absolute_error);
        let energy_scale = qubo_energy.abs().max(ising_energy.abs());
        let allowed_error = config.absolute_tolerance + config.relative_tolerance * energy_scale;
        if absolute_error > allowed_error {
            return Err(QuantumContractError::EnergyMismatch {
                assignment_index,
                qubo_energy,
                ising_energy,
                absolute_error,
                allowed_error,
            });
        }
    }

    Ok(EnergyEquivalenceReport {
        schema_version: ENERGY_EQUIVALENCE_SCHEMA_VERSION.to_owned(),
        variable_count,
        assignment_space_size,
        assignment_limit: config.assignment_limit,
        assignments_checked,
        exhaustive,
        maximum_absolute_error,
        absolute_tolerance: config.absolute_tolerance,
        relative_tolerance: config.relative_tolerance,
        mapping: "x = (1-z)/2; z = 1-2x".to_owned(),
        limitation: if exhaustive {
            "all logical assignments were checked; this does not evaluate hardware execution"
                .to_owned()
        } else {
            "bounded deterministic assignments were checked; this is not an exhaustive proof or a quantum-advantage claim"
                .to_owned()
        },
    })
}

fn assignment_space_size(variable_count: usize) -> Option<u64> {
    u32::try_from(variable_count)
        .ok()
        .filter(|count| *count < u64::BITS)
        .map(|count| 1_u64 << count)
}

fn generated_assignment(index: usize, variable_count: usize, exhaustive: bool) -> Vec<bool> {
    if exhaustive {
        return (0..variable_count)
            .map(|bit| ((index >> bit) & 1) == 1)
            .collect();
    }
    match index {
        0 => vec![false; variable_count],
        1 => vec![true; variable_count],
        2 => (0..variable_count).map(|bit| bit % 2 == 1).collect(),
        3 => (0..variable_count).map(|bit| bit % 2 == 0).collect(),
        _ => (0..variable_count)
            .map(|bit| {
                let block = bit / u64::BITS as usize;
                let word = splitmix64(
                    u64::try_from(index).unwrap_or(u64::MAX)
                        ^ u64::try_from(block)
                            .unwrap_or(u64::MAX)
                            .wrapping_mul(0x9e37_79b9_7f4a_7c15),
                );
                ((word >> (bit % u64::BITS as usize)) & 1) == 1
            })
            .collect(),
    }
}

const fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
