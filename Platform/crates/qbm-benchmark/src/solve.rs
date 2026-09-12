//! Bounded classical baselines and common result scoring.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{BenchmarkError, BenchmarkProfile, OPTIMIZATION_RESULT_SCHEMA_VERSION};

/// Classical solver used to produce an optimization result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolverKind {
    /// Exhaustive enumeration with a proof of optimality under configured bounds.
    Exact,
    /// Deterministic marginal weighted-coverage-per-cost baseline.
    Greedy,
    /// Seeded deterministic simulated-annealing baseline.
    SimulatedAnnealing,
    /// Seeded deterministic one-flip tabu-search baseline.
    TabuSearch,
}

/// Per-run optimization controls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationRequest {
    /// Solver to execute.
    pub solver: SolverKind,
    /// Optional additional panel-size ceiling.
    pub max_inputs: Option<usize>,
    /// Optional required weighted-coverage fraction in `(0, 1]`.
    pub coverage_floor: Option<f64>,
    /// Explicit seed used by stochastic-looking deterministic baselines.
    pub seed: u64,
    /// Iterations used by annealing/tabu; ignored by exact and greedy solvers.
    pub iterations: usize,
}

impl OptimizationRequest {
    /// Construct a deterministic request with no additional constraints.
    #[must_use]
    pub const fn new(solver: SolverKind) -> Self {
        Self {
            solver,
            max_inputs: None,
            coverage_floor: None,
            seed: 0,
            iterations: 0,
        }
    }
}

/// Hard resource ceilings for local deterministic solvers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolverConfig {
    /// Largest input count accepted by exhaustive bit-mask enumeration.
    pub exact_max_inputs: usize,
    /// Largest number of exact bit-mask states evaluated.
    pub max_exact_states: u64,
    /// Default iteration count used by coverage/minimum-panel helpers.
    pub default_heuristic_iterations: usize,
    /// Hard iteration ceiling for annealing and tabu search.
    pub max_heuristic_iterations: usize,
    /// Number of iterations a flipped variable remains tabu.
    pub tabu_tenure: usize,
    /// Default deterministic seed used by helper operations.
    pub default_seed: u64,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            exact_max_inputs: 20,
            max_exact_states: 1 << 20,
            default_heuristic_iterations: 10_000,
            max_heuristic_iterations: 1_000_000,
            tabu_tenure: 11,
            default_seed: 0x5142_4d45_4442_454e,
        }
    }
}

/// Native, constraint-aware score of one selected input panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionScore {
    /// Sorted selected input identities.
    pub selected_inputs: Vec<String>,
    /// Sorted outcomes covered at least once.
    pub covered_outcomes: Vec<String>,
    /// Sorted outcomes not covered.
    pub uncovered_outcomes: Vec<String>,
    /// Number of selected inputs.
    pub selected_count: usize,
    /// Sum of selected input costs.
    pub total_cost: f64,
    /// Sum of covered outcome weights.
    pub covered_weight: f64,
    /// Sum of all outcome weights.
    pub total_outcome_weight: f64,
    /// `covered_weight / total_outcome_weight`.
    pub coverage_fraction: f64,
    /// Whether all profile-level constraints are satisfied.
    pub profile_constraints_satisfied: bool,
    /// Sorted, deterministic descriptions of violated profile constraints.
    pub constraint_violations: Vec<String>,
}

/// Common output shared by all classical baselines and QUBO rescoring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationResult {
    /// Output schema identifier.
    pub schema_version: String,
    /// Stable profile identity.
    pub profile_id: String,
    /// Algorithm that produced the result.
    pub solver: SolverKind,
    /// True only for exhaustive search completed within the declared bounds.
    pub optimality_proven: bool,
    /// Seed recorded for reproducibility; absent for exact and greedy runs.
    pub deterministic_seed: Option<u64>,
    /// Iterations actually completed by a heuristic.
    pub iterations_completed: usize,
    /// Candidate selections scored by this run.
    pub evaluated_candidates: u64,
    /// Request used to run this solver.
    pub request: OptimizationRequest,
    /// Native biomedical score of the selected panel.
    pub score: SelectionScore,
    /// Honest algorithm/scope limitations, in deterministic order.
    pub limitations: Vec<String>,
}

/// Coverage result at one requested panel-size ceiling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoveragePoint {
    /// Requested maximum panel size.
    pub k: usize,
    /// Solver result under that ceiling.
    pub result: OptimizationResult,
}

/// Score an explicit selected panel using the profile's native biomedical model.
///
/// Profile constraint violations are returned in the score rather than hidden;
/// unknown or duplicate input identities are rejected.
pub fn evaluate_selection(
    profile: &BenchmarkProfile,
    selected_inputs: &[String],
) -> Result<SelectionScore, BenchmarkError> {
    profile.validate()?;
    let mut selected = selected_inputs.to_vec();
    selected.sort();
    if selected.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(BenchmarkError::InvalidRequest(
            "selected_inputs contains duplicate identities".to_owned(),
        ));
    }
    let known: BTreeSet<_> = profile
        .inputs
        .iter()
        .map(|input| input.id.as_str())
        .collect();
    if let Some(unknown) = selected
        .iter()
        .find(|input| !known.contains(input.as_str()))
    {
        return Err(BenchmarkError::InvalidRequest(format!(
            "selected_inputs contains unknown identity {unknown:?}"
        )));
    }
    Ok(score_unchecked(profile, &selected))
}

/// Solve weighted maximum coverage under profile and request constraints.
pub fn solve(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
    config: &SolverConfig,
) -> Result<OptimizationResult, BenchmarkError> {
    profile.validate()?;
    validate_request(profile, request, config)?;
    match request.solver {
        SolverKind::Exact => exact_solve(profile, request, config, ExactGoal::MaxCoverage),
        SolverKind::Greedy => greedy_solve(profile, request),
        SolverKind::SimulatedAnnealing => annealing_solve(profile, request),
        SolverKind::TabuSearch => tabu_solve(profile, request, config.tabu_tenure),
    }
}

/// Compute coverage at strictly increasing, positive panel-size ceilings.
pub fn coverage_at_k(
    profile: &BenchmarkProfile,
    panel_sizes: &[usize],
    solver: SolverKind,
    config: &SolverConfig,
) -> Result<Vec<CoveragePoint>, BenchmarkError> {
    profile.validate()?;
    if panel_sizes.is_empty()
        || panel_sizes.contains(&0)
        || !panel_sizes.windows(2).all(|pair| pair[0] < pair[1])
    {
        return Err(BenchmarkError::InvalidRequest(
            "panel_sizes must be non-empty, positive, strictly increasing, and unique".to_owned(),
        ));
    }
    if panel_sizes.iter().any(|size| *size > profile.inputs.len()) {
        return Err(BenchmarkError::InvalidRequest(
            "a requested panel size exceeds the profile input count".to_owned(),
        ));
    }

    panel_sizes
        .iter()
        .map(|k| {
            let request = OptimizationRequest {
                solver,
                max_inputs: Some(*k),
                coverage_floor: None,
                seed: config.default_seed,
                iterations: helper_iterations(solver, config),
            };
            solve(profile, &request, config).map(|result| CoveragePoint { k: *k, result })
        })
        .collect()
}

/// Find a smallest panel meeting a weighted-coverage floor.
///
/// Exact mode proves minimum cardinality. Other modes are deterministic
/// baselines and report `optimality_proven = false`.
pub fn minimum_panel(
    profile: &BenchmarkProfile,
    coverage_floor: f64,
    solver: SolverKind,
    config: &SolverConfig,
) -> Result<OptimizationResult, BenchmarkError> {
    profile.validate()?;
    validate_floor(coverage_floor)?;
    if solver == SolverKind::Exact {
        let request = OptimizationRequest {
            solver,
            max_inputs: None,
            coverage_floor: Some(coverage_floor),
            seed: config.default_seed,
            iterations: 0,
        };
        validate_request(profile, &request, config)?;
        return exact_solve(profile, &request, config, ExactGoal::MinimumPanel);
    }

    let maximum = effective_max_inputs(profile, None);
    for size in profile.constraints.min_selected.max(1)..=maximum {
        let request = OptimizationRequest {
            solver,
            max_inputs: Some(size),
            coverage_floor: Some(coverage_floor),
            seed: config.default_seed,
            iterations: helper_iterations(solver, config),
        };
        match solve(profile, &request, config) {
            Ok(result) => return Ok(result),
            Err(BenchmarkError::Infeasible(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Err(BenchmarkError::Infeasible(format!(
        "no {solver:?} baseline selection met coverage floor {coverage_floor}"
    )))
}

fn helper_iterations(solver: SolverKind, config: &SolverConfig) -> usize {
    match solver {
        SolverKind::SimulatedAnnealing | SolverKind::TabuSearch => {
            config.default_heuristic_iterations
        }
        SolverKind::Exact | SolverKind::Greedy => 0,
    }
}

fn validate_request(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
    config: &SolverConfig,
) -> Result<(), BenchmarkError> {
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
    if let Some(floor) = request.coverage_floor {
        validate_floor(floor)?;
    }
    if matches!(
        request.solver,
        SolverKind::SimulatedAnnealing | SolverKind::TabuSearch
    ) && (request.iterations == 0 || request.iterations > config.max_heuristic_iterations)
    {
        return Err(BenchmarkError::InvalidRequest(format!(
            "heuristic iterations must be within 1..={}",
            config.max_heuristic_iterations
        )));
    }
    Ok(())
}

fn validate_floor(floor: f64) -> Result<(), BenchmarkError> {
    if !(floor.is_finite() && 0.0 < floor && floor <= 1.0) {
        return Err(BenchmarkError::InvalidRequest(
            "coverage_floor must be finite and in (0, 1]".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExactGoal {
    MaxCoverage,
    MinimumPanel,
}

fn exact_solve(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
    config: &SolverConfig,
    goal: ExactGoal,
) -> Result<OptimizationResult, BenchmarkError> {
    let count = profile.inputs.len();
    let required_states = u32::try_from(count)
        .ok()
        .and_then(|shift| 1_u64.checked_shl(shift))
        .unwrap_or(u64::MAX);
    if count > config.exact_max_inputs || required_states > config.max_exact_states {
        return Err(BenchmarkError::ExactLimit {
            required_states,
            maximum_states: config.max_exact_states,
        });
    }

    let mut best: Option<SelectionScore> = None;
    let mut evaluated = 0_u64;
    for mask in 0..required_states {
        evaluated += 1;
        let selected: Vec<_> = profile
            .inputs
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1_u64 << index) != 0)
            .map(|(_, input)| input.id.clone())
            .collect();
        let score = score_unchecked(profile, &selected);
        if !request_feasible(profile, request, &score) {
            continue;
        }
        let replace = best.as_ref().is_none_or(|current| match goal {
            ExactGoal::MaxCoverage => better_max_coverage(&score, current),
            ExactGoal::MinimumPanel => better_minimum_panel(&score, current),
        });
        if replace {
            best = Some(score);
        }
    }
    let score = best.ok_or_else(|| {
        BenchmarkError::Infeasible("no selection satisfies every declared constraint".to_owned())
    })?;
    Ok(OptimizationResult {
        schema_version: OPTIMIZATION_RESULT_SCHEMA_VERSION.to_owned(),
        profile_id: profile.profile_id.clone(),
        solver: SolverKind::Exact,
        optimality_proven: true,
        deterministic_seed: None,
        iterations_completed: 0,
        evaluated_candidates: evaluated,
        request: request.clone(),
        score,
        limitations: vec![
            "proof applies only to the explicit binary incidence profile and declared constraints"
                .to_owned(),
        ],
    })
}

fn greedy_solve(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
) -> Result<OptimizationResult, BenchmarkError> {
    let (score, evaluated) = greedy_candidate(profile, request)?;
    Ok(OptimizationResult {
        schema_version: OPTIMIZATION_RESULT_SCHEMA_VERSION.to_owned(),
        profile_id: profile.profile_id.clone(),
        solver: SolverKind::Greedy,
        optimality_proven: false,
        deterministic_seed: None,
        iterations_completed: 0,
        evaluated_candidates: evaluated,
        request: request.clone(),
        score,
        limitations: vec![
            "greedy weighted-gain-per-cost selection does not prove optimality".to_owned(),
        ],
    })
}

fn greedy_candidate(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
) -> Result<(SelectionScore, u64), BenchmarkError> {
    let excluded: BTreeSet<_> = profile
        .constraints
        .excluded_inputs
        .iter()
        .map(String::as_str)
        .collect();
    let mut selected = profile.constraints.required_inputs.clone();
    let maximum = effective_max_inputs(profile, request.max_inputs);
    let mut evaluated = 0_u64;

    loop {
        let current = score_unchecked(profile, &selected);
        let must_add = selected.len() < profile.constraints.min_selected;
        let needs_floor = request
            .coverage_floor
            .is_some_and(|floor| current.coverage_fraction + f64::EPSILON < floor);
        let needs_required_outcome = profile
            .constraints
            .required_outcomes
            .iter()
            .any(|outcome| !current.covered_outcomes.contains(outcome));
        if selected.len() >= maximum {
            break;
        }

        let mut best: Option<(f64, SelectionScore, String)> = None;
        for input in &profile.inputs {
            if excluded.contains(input.id.as_str()) || selected.contains(&input.id) {
                continue;
            }
            let mut candidate_ids = selected.clone();
            candidate_ids.push(input.id.clone());
            candidate_ids.sort();
            let candidate = score_unchecked(profile, &candidate_ids);
            evaluated += 1;
            if violates_budget(profile, &candidate) {
                continue;
            }
            let gain = candidate.covered_weight - current.covered_weight;
            let ratio = gain / input.cost;
            let replace = best
                .as_ref()
                .is_none_or(|(best_ratio, best_score, best_id)| {
                    ratio.total_cmp(best_ratio) == Ordering::Greater
                        || (ratio.total_cmp(best_ratio) == Ordering::Equal
                            && (better_max_coverage(&candidate, best_score)
                                || (candidate == *best_score && input.id < *best_id)))
                });
            if replace {
                best = Some((ratio, candidate, input.id.clone()));
            }
        }

        let Some((ratio, candidate, _)) = best else {
            break;
        };
        if ratio <= 0.0 && !must_add && !needs_floor && !needs_required_outcome {
            break;
        }
        selected = candidate.selected_inputs;
    }

    let score = score_unchecked(profile, &selected);
    if !request_feasible(profile, request, &score) {
        return Err(BenchmarkError::Infeasible(
            "the greedy baseline did not find a feasible selection".to_owned(),
        ));
    }
    Ok((score, evaluated))
}

#[allow(clippy::cast_precision_loss)] // Iteration progress is intentionally normalized as f64.
fn annealing_solve(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
) -> Result<OptimizationResult, BenchmarkError> {
    let (initial, mut evaluated) = greedy_candidate(profile, request)?;
    let mut current = initial.clone();
    let mut best = initial;
    let mutable = mutable_input_ids(profile);
    let total_weight = best.total_outcome_weight.max(f64::EPSILON);
    let mut rng = DeterministicRng::new(request.seed);

    for iteration in 0..request.iterations {
        if mutable.is_empty() {
            break;
        }
        let input = &mutable[rng.index(mutable.len())];
        let candidate_ids = toggled(&current.selected_inputs, input);
        let candidate = score_unchecked(profile, &candidate_ids);
        evaluated += 1;
        if !request_feasible(profile, request, &candidate) {
            continue;
        }

        let improvement = utility(&candidate) - utility(&current);
        let temperature = 1.0 - (iteration as f64 / request.iterations as f64);
        let loss = (-improvement / total_weight).max(0.0);
        let acceptance = if improvement >= 0.0 {
            1.0
        } else {
            temperature / (temperature + loss + f64::EPSILON)
        };
        if rng.unit() < acceptance {
            current = candidate;
            if better_max_coverage(&current, &best) {
                best = current.clone();
            }
        }
    }

    Ok(OptimizationResult {
        schema_version: OPTIMIZATION_RESULT_SCHEMA_VERSION.to_owned(),
        profile_id: profile.profile_id.clone(),
        solver: SolverKind::SimulatedAnnealing,
        optimality_proven: false,
        deterministic_seed: Some(request.seed),
        iterations_completed: request.iterations,
        evaluated_candidates: evaluated,
        request: request.clone(),
        score: best,
        limitations: vec![
            "seeded annealing is a reproducible heuristic and does not prove optimality".to_owned(),
        ],
    })
}

fn tabu_solve(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
    tabu_tenure: usize,
) -> Result<OptimizationResult, BenchmarkError> {
    let (initial, mut evaluated) = greedy_candidate(profile, request)?;
    let mut current = initial.clone();
    let mut best = initial;
    let mutable = mutable_input_ids(profile);
    let mut tabu_until: BTreeMap<&str, usize> = BTreeMap::new();

    for iteration in 0..request.iterations {
        let mut next: Option<(SelectionScore, &str)> = None;
        for input in &mutable {
            let candidate_ids = toggled(&current.selected_inputs, input);
            let candidate = score_unchecked(profile, &candidate_ids);
            evaluated += 1;
            if !request_feasible(profile, request, &candidate) {
                continue;
            }
            let is_tabu = tabu_until
                .get(input.as_str())
                .is_some_and(|until| *until > iteration);
            let aspiration = better_max_coverage(&candidate, &best);
            if is_tabu && !aspiration {
                continue;
            }
            if next.as_ref().is_none_or(|(score, id)| {
                better_max_coverage(&candidate, score)
                    || (candidate == *score && input.as_str() < *id)
            }) {
                next = Some((candidate, input));
            }
        }
        let Some((candidate, flipped)) = next else {
            break;
        };
        current = candidate;
        tabu_until.insert(
            flipped,
            iteration.saturating_add(tabu_tenure).saturating_add(1),
        );
        if better_max_coverage(&current, &best) {
            best = current.clone();
        }
    }

    Ok(OptimizationResult {
        schema_version: OPTIMIZATION_RESULT_SCHEMA_VERSION.to_owned(),
        profile_id: profile.profile_id.clone(),
        solver: SolverKind::TabuSearch,
        optimality_proven: false,
        deterministic_seed: Some(request.seed),
        iterations_completed: request.iterations,
        evaluated_candidates: evaluated,
        request: request.clone(),
        score: best,
        limitations: vec![
            "one-flip tabu search is a reproducible heuristic and does not prove optimality"
                .to_owned(),
        ],
    })
}

fn score_unchecked(profile: &BenchmarkProfile, selected: &[String]) -> SelectionScore {
    let selected_set: BTreeSet<_> = selected.iter().map(String::as_str).collect();
    let covered_set: BTreeSet<_> = profile
        .relationships
        .iter()
        .filter(|relationship| selected_set.contains(relationship.input_id.as_str()))
        .map(|relationship| relationship.outcome_id.as_str())
        .collect();
    let covered_outcomes: Vec<_> = profile
        .outcomes
        .iter()
        .filter(|outcome| covered_set.contains(outcome.id.as_str()))
        .map(|outcome| outcome.id.clone())
        .collect();
    let uncovered_outcomes: Vec<_> = profile
        .outcomes
        .iter()
        .filter(|outcome| !covered_set.contains(outcome.id.as_str()))
        .map(|outcome| outcome.id.clone())
        .collect();
    let total_cost = profile
        .inputs
        .iter()
        .filter(|input| selected_set.contains(input.id.as_str()))
        .map(|input| input.cost)
        .sum();
    let covered_weight = profile
        .outcomes
        .iter()
        .filter(|outcome| covered_set.contains(outcome.id.as_str()))
        .map(|outcome| outcome.weight)
        .sum();
    let total_outcome_weight: f64 = profile.outcomes.iter().map(|outcome| outcome.weight).sum();
    let mut score = SelectionScore {
        selected_inputs: selected.to_vec(),
        covered_outcomes,
        uncovered_outcomes,
        selected_count: selected.len(),
        total_cost,
        covered_weight,
        total_outcome_weight,
        coverage_fraction: covered_weight / total_outcome_weight,
        profile_constraints_satisfied: false,
        constraint_violations: Vec::new(),
    };
    score.constraint_violations = profile_violations(profile, &score);
    score.profile_constraints_satisfied = score.constraint_violations.is_empty();
    score
}

fn profile_violations(profile: &BenchmarkProfile, score: &SelectionScore) -> Vec<String> {
    let selected: BTreeSet<_> = score.selected_inputs.iter().map(String::as_str).collect();
    let covered: BTreeSet<_> = score.covered_outcomes.iter().map(String::as_str).collect();
    let mut violations = Vec::new();
    if score.selected_count < profile.constraints.min_selected {
        violations.push("selected_count_below_minimum".to_owned());
    }
    if profile
        .constraints
        .max_selected
        .is_some_and(|maximum| score.selected_count > maximum)
    {
        violations.push("selected_count_above_maximum".to_owned());
    }
    if profile
        .constraints
        .max_total_cost
        .is_some_and(|maximum| score.total_cost > maximum + f64::EPSILON)
    {
        violations.push("total_cost_above_maximum".to_owned());
    }
    if profile
        .constraints
        .required_inputs
        .iter()
        .any(|input| !selected.contains(input.as_str()))
    {
        violations.push("required_input_missing".to_owned());
    }
    if profile
        .constraints
        .excluded_inputs
        .iter()
        .any(|input| selected.contains(input.as_str()))
    {
        violations.push("excluded_input_selected".to_owned());
    }
    if profile
        .constraints
        .required_outcomes
        .iter()
        .any(|outcome| !covered.contains(outcome.as_str()))
    {
        violations.push("required_outcome_uncovered".to_owned());
    }
    violations
}

fn request_feasible(
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
    score: &SelectionScore,
) -> bool {
    score.profile_constraints_satisfied
        && request
            .max_inputs
            .is_none_or(|maximum| score.selected_count <= maximum)
        && request
            .coverage_floor
            .is_none_or(|floor| score.coverage_fraction + f64::EPSILON >= floor)
        && !violates_budget(profile, score)
}

fn violates_budget(profile: &BenchmarkProfile, score: &SelectionScore) -> bool {
    profile
        .constraints
        .max_total_cost
        .is_some_and(|maximum| score.total_cost > maximum + f64::EPSILON)
}

fn effective_max_inputs(profile: &BenchmarkProfile, request_maximum: Option<usize>) -> usize {
    [
        Some(profile.inputs.len()),
        profile.constraints.max_selected,
        request_maximum,
    ]
    .into_iter()
    .flatten()
    .min()
    .expect("input count always supplies a maximum")
}

fn better_max_coverage(candidate: &SelectionScore, current: &SelectionScore) -> bool {
    candidate.covered_weight.total_cmp(&current.covered_weight) == Ordering::Greater
        || (candidate.covered_weight.total_cmp(&current.covered_weight) == Ordering::Equal
            && (candidate.total_cost.total_cmp(&current.total_cost) == Ordering::Less
                || (candidate.total_cost.total_cmp(&current.total_cost) == Ordering::Equal
                    && (candidate.selected_count < current.selected_count
                        || (candidate.selected_count == current.selected_count
                            && candidate.selected_inputs < current.selected_inputs)))))
}

fn better_minimum_panel(candidate: &SelectionScore, current: &SelectionScore) -> bool {
    candidate.selected_count < current.selected_count
        || (candidate.selected_count == current.selected_count
            && (candidate.covered_weight.total_cmp(&current.covered_weight) == Ordering::Greater
                || (candidate.covered_weight.total_cmp(&current.covered_weight)
                    == Ordering::Equal
                    && (candidate.total_cost.total_cmp(&current.total_cost) == Ordering::Less
                        || (candidate.total_cost.total_cmp(&current.total_cost)
                            == Ordering::Equal
                            && candidate.selected_inputs < current.selected_inputs)))))
}

fn mutable_input_ids(profile: &BenchmarkProfile) -> Vec<String> {
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
    profile
        .inputs
        .iter()
        .filter(|input| {
            !required.contains(input.id.as_str()) && !excluded.contains(input.id.as_str())
        })
        .map(|input| input.id.clone())
        .collect()
}

fn toggled(selected: &[String], input: &str) -> Vec<String> {
    let mut result = selected.to_vec();
    match result.binary_search_by(|value| value.as_str().cmp(input)) {
        Ok(index) => {
            result.remove(index);
        }
        Err(index) => result.insert(index, input.to_owned()),
    }
    result
}

#[allow(clippy::cast_precision_loss)] // Count is a deterministic, tiny tie-breaker in an f64 utility.
fn utility(score: &SelectionScore) -> f64 {
    score.covered_weight - score.total_cost * 1e-9 - score.selected_count as f64 * 1e-12
}

#[derive(Debug, Clone, Copy)]
struct DeterministicRng(u64);

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9e37_79b9_7f4a_7c15
        } else {
            seed
        })
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    #[allow(clippy::cast_possible_truncation)] // The remainder is strictly below the usize-valued length.
    fn index(&mut self, length: usize) -> usize {
        (self.next() % length as u64) as usize
    }

    #[allow(clippy::cast_precision_loss)] // This is the standard conversion of 53 random bits to [0, 1).
    fn unit(&mut self) -> f64 {
        let mantissa = self.next() >> 11;
        mantissa as f64 / (1_u64 << 53) as f64
    }
}
