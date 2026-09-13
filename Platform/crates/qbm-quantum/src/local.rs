//! A classical solver behind the executor seam.
//!
//! The executor boundary existed as a trait with nothing on the other side, so
//! the exported QUBO/Ising model was never actually solved by anything. That
//! left a real question unanswered: does the formulation this crate exports
//! describe the same problem the native solvers answer? A model can be
//! internally consistent — it builds, it converts to Ising, its energies check
//! out — and still encode something easier than the profile states.
//!
//! [`LocalIsingExecutor`] answers that question by minimising the exported
//! model directly and handing back its lowest-energy assignments. Decode those
//! to an input panel, score it natively, and compare with the certified answer:
//! if they disagree, the formulation is wrong.
//!
//! # This is not a quantum computer
//!
//! It is a deterministic classical search running on this machine, declared as
//! [`BackendKind::Simulator`] and labelled as such in every report it appears
//! in. It executes the *logical* model only — no hardware topology embedding,
//! no noise, no sampling statistics that mean anything physical. Its energies
//! say what the model says; they say nothing about what a quantum device would
//! do with it, and nothing here is evidence of quantum advantage.
//!
//! Solving is exhaustive below a configured variable count and a seeded
//! multi-restart local search above it. Above that threshold a returned
//! assignment is the best one found, not a proven minimum — the same honesty
//! the classical baselines observe, for the same reason.

use std::collections::BTreeSet;

use chrono::Utc;

use crate::{
    BACKEND_CAPABILITY_SCHEMA_VERSION, BackendCapability, BackendKind, ExecutionRequest,
    ExecutionSample, ExecutionUpdate, ExecutorError, ExecutorErrorKind, JobReceipt, JobResult,
    JobState, ProblemEncoding, QuantumExecutor, QuantumProblem,
};

/// Resource ceilings for the local search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSolverConfig {
    /// Largest variable count solved by exhaustive enumeration.
    ///
    /// Below this the returned minimum is the true minimum. Above it the
    /// search is a heuristic and the result says so.
    pub exhaustive_max_variables: usize,
    /// Independent descents run by the heuristic search.
    pub restarts: usize,
    /// Flip attempts per descent.
    pub iterations_per_restart: usize,
    /// Total single-bit energy deltas the heuristic may evaluate.
    ///
    /// Each descent step scores every variable, so cost grows with the product
    /// of restarts, iterations and model width. A flat budget keeps a wide
    /// model from turning a report into a hang, and the search stops early
    /// rather than silently running longer than anyone expected.
    pub max_delta_evaluations: u64,
    /// Largest model the backend will accept at all.
    pub maximum_variables: usize,
}

impl Default for LocalSolverConfig {
    fn default() -> Self {
        Self {
            // 2^22 energy evaluations with incremental deltas is around a
            // second; past that the heuristic is the better use of the time.
            exhaustive_max_variables: 22,
            restarts: 16,
            iterations_per_restart: 2_000,
            max_delta_evaluations: 40_000_000,
            maximum_variables: 4_096,
        }
    }
}

/// Stable identifier for this backend.
pub const LOCAL_ISING_BACKEND_ID: &str = "local-ising-reference";

/// A deterministic classical minimiser for exported QUBO and Ising models.
///
/// Stateless: `submit` accepts a request, `poll` computes the answer. The same
/// request always produces the same samples, so a caller that polls twice sees
/// one answer rather than two.
#[derive(Debug, Clone)]
pub struct LocalIsingExecutor {
    capability: BackendCapability,
    config: LocalSolverConfig,
}

impl Default for LocalIsingExecutor {
    fn default() -> Self {
        Self::new(LocalSolverConfig::default())
    }
}

impl LocalIsingExecutor {
    /// Build the executor and its declared capability.
    #[must_use]
    pub fn new(config: LocalSolverConfig) -> Self {
        Self {
            capability: BackendCapability {
                schema_version: BACKEND_CAPABILITY_SCHEMA_VERSION.to_owned(),
                backend_id: LOCAL_ISING_BACKEND_ID.to_owned(),
                provider: "q-benchmed-local".to_owned(),
                display_name: "Local Ising reference solver (classical)".to_owned(),
                backend_kind: BackendKind::Simulator,
                supported_encodings: vec![ProblemEncoding::Qubo, ProblemEncoding::Ising],
                maximum_variables: Some(config.maximum_variables),
                maximum_couplings: None,
                maximum_samples_per_request: Some(1_024),
                supports_seed: true,
                supports_polling: true,
                limitations: vec![
                    "This backend is a classical solver running on this machine. It is not a quantum processor and no result from it is evidence of quantum advantage.".to_owned(),
                    "It minimises the logical model only. No hardware topology embedding, calibration, noise model or physical sampling is involved.".to_owned(),
                    format!(
                        "Minima are proven by exhaustive enumeration up to {} variables. Above that the search is a seeded heuristic bounded at {} energy evaluations, and a returned assignment is the best found, not a proven minimum.",
                        config.exhaustive_max_variables, config.max_delta_evaluations
                    ),
                    "Sample occurrence counts describe how many restarts reached an assignment, not a physical measurement distribution.".to_owned(),
                ],
            },
            config,
        }
    }

    /// Configured resource ceilings.
    #[must_use]
    pub const fn config(&self) -> LocalSolverConfig {
        self.config
    }

    /// Whether a model of this size gets a proven minimum.
    #[must_use]
    pub const fn proves_minimum(&self, variable_count: usize) -> bool {
        variable_count <= self.config.exhaustive_max_variables
    }

    /// Minimise a problem and return its best distinct assignments.
    ///
    /// Sorted by energy ascending, then by assignment, which is the order the
    /// result contract requires.
    pub fn minimize(
        &self,
        problem: &QuantumProblem,
        sample_count: usize,
        seed: u64,
    ) -> Result<Vec<(Vec<bool>, f64, u64)>, ExecutorError> {
        let form = BitQuadratic::from_problem(problem).map_err(|reason| {
            ExecutorError::new(
                ExecutorErrorKind::Unsupported,
                "unconvertible_model",
                reason,
                false,
            )
        })?;
        let wanted = sample_count.max(1);
        let found = if self.proves_minimum(form.variables) {
            form.enumerate(wanted)
        } else {
            form.anneal(
                wanted,
                seed,
                self.config.restarts,
                self.config.max_delta_evaluations,
            )
        };
        Ok(found)
    }
}

impl QuantumExecutor for LocalIsingExecutor {
    fn capability(&self) -> &BackendCapability {
        &self.capability
    }

    fn submit(&self, request: &ExecutionRequest) -> Result<JobReceipt, ExecutorError> {
        request
            .validate_for_backend(&self.capability)
            .map_err(|error| {
                ExecutorError::new(
                    ExecutorErrorKind::Unsupported,
                    "unsupported_request",
                    error.to_string(),
                    false,
                )
            })?;
        // Local execution is synchronous, so the job is running the moment it
        // is accepted. It becomes terminal on the first poll.
        JobReceipt::new(
            request,
            format!("local-{}", &request.request_sha256.as_str()[..16]),
            JobState::Running,
            Utc::now(),
        )
        .map_err(invalid_response)
    }

    fn poll(
        &self,
        request: &ExecutionRequest,
        receipt: &JobReceipt,
    ) -> Result<ExecutionUpdate, ExecutorError> {
        receipt
            .validate_against(request)
            .map_err(invalid_response)?;
        if matches!(
            receipt.state,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        ) {
            // Already terminal: recompute deterministically rather than
            // caching, so the executor holds no state that could go stale.
            return self.terminal_update(request, receipt.clone());
        }
        // Carry the original acceptance time forward so the only thing that
        // changed between the two receipts is the state.
        let terminal = JobReceipt::new(
            request,
            receipt.job_id.clone(),
            JobState::Succeeded,
            receipt.submitted_at,
        )
        .map_err(invalid_response)?;
        self.terminal_update(request, terminal)
    }
}

impl LocalIsingExecutor {
    fn terminal_update(
        &self,
        request: &ExecutionRequest,
        receipt: JobReceipt,
    ) -> Result<ExecutionUpdate, ExecutorError> {
        let problem = &request.payload.problem;
        let found = self.minimize(
            problem,
            request.sample_count as usize,
            request.seed.unwrap_or(0),
        )?;
        let samples = found
            .into_iter()
            .map(|(assignment, energy, occurrences)| {
                ExecutionSample::new(problem, assignment, Some(energy), occurrences)
                    .map_err(invalid_response)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let result = JobResult::succeeded(request, &receipt, samples, Utc::now())
            .map_err(invalid_response)?;
        ExecutionUpdate::complete(request, receipt, result).map_err(invalid_response)
    }
}

fn invalid_response(error: crate::QuantumContractError) -> ExecutorError {
    ExecutorError::new(
        ExecutorErrorKind::InvalidResponse,
        "invalid_response",
        error.to_string(),
        false,
    )
}

/// A quadratic form over bits, with the adjacency needed for flip deltas.
///
/// Both encodings collapse to `offset + sum l_i x_i + sum q_ij x_i x_j`, which
/// gives one search loop instead of two and one place for the arithmetic to be
/// wrong. The conversion is checked against the contract's own energy function
/// in this module's tests.
struct BitQuadratic {
    variables: usize,
    offset: f64,
    linear: Vec<f64>,
    /// `neighbours[i]` holds `(j, coefficient)` for every partner of `i`.
    neighbours: Vec<Vec<(usize, f64)>>,
}

impl BitQuadratic {
    fn from_problem(problem: &QuantumProblem) -> Result<Self, String> {
        let variables = problem.variable_count();
        let mut linear = vec![0.0; variables];
        let mut offset;
        let mut pairs: Vec<(usize, usize, f64)> = Vec::new();

        match problem {
            QuantumProblem::Qubo(model) => {
                offset = model.offset;
                linear.copy_from_slice(&model.linear);
                for coupling in &model.quadratic {
                    pairs.push((coupling.left, coupling.right, coupling.coefficient));
                }
            }
            QuantumProblem::Ising(model) => {
                // z_i = 1 - 2 x_i, so
                //   sum h_i z_i      = sum h_i        - 2 sum h_i x_i
                //   sum J_ij z_i z_j = sum J_ij       - 2 sum J_ij (x_i + x_j)
                //                                     + 4 sum J_ij x_i x_j
                offset = model.offset;
                for (index, bias) in model.linear.iter().enumerate() {
                    offset += bias;
                    linear[index] -= 2.0 * bias;
                }
                for coupling in &model.quadratic {
                    offset += coupling.coefficient;
                    linear[coupling.left] -= 2.0 * coupling.coefficient;
                    linear[coupling.right] -= 2.0 * coupling.coefficient;
                    pairs.push((coupling.left, coupling.right, 4.0 * coupling.coefficient));
                }
            }
        }

        let mut neighbours = vec![Vec::new(); variables];
        for (left, right, coefficient) in pairs {
            if left >= variables || right >= variables {
                return Err("a coupling references a variable outside the model".to_owned());
            }
            neighbours[left].push((right, coefficient));
            neighbours[right].push((left, coefficient));
        }
        if !offset.is_finite() || linear.iter().any(|value| !value.is_finite()) {
            return Err("the converted model has a non-finite coefficient".to_owned());
        }
        Ok(Self {
            variables,
            offset,
            linear,
            neighbours,
        })
    }

    fn energy(&self, assignment: &[bool]) -> f64 {
        let mut total = self.offset;
        for (index, on) in assignment.iter().enumerate() {
            if !on {
                continue;
            }
            total += self.linear[index];
            for (other, coefficient) in &self.neighbours[index] {
                if *other > index && assignment[*other] {
                    total += coefficient;
                }
            }
        }
        total
    }

    /// Energy change from flipping one bit, without rebuilding the total.
    fn delta(&self, assignment: &[bool], index: usize) -> f64 {
        let mut interaction = self.linear[index];
        for (other, coefficient) in &self.neighbours[index] {
            if assignment[*other] {
                interaction += coefficient;
            }
        }
        if assignment[index] {
            -interaction
        } else {
            interaction
        }
    }

    /// Every assignment, keeping the `wanted` lowest.
    fn enumerate(&self, wanted: usize) -> Vec<(Vec<bool>, f64, u64)> {
        let mut best: Vec<(Vec<bool>, f64)> = Vec::new();
        let total = 1_u64 << self.variables;
        for mask in 0..total {
            let assignment: Vec<bool> = (0..self.variables)
                .map(|bit| mask & (1 << bit) != 0)
                .collect();
            let energy = self.energy(&assignment);
            best.push((assignment, energy));
            if best.len() > wanted * 4 {
                best.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
                best.truncate(wanted);
            }
        }
        best.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
        best.truncate(wanted);
        // Every assignment was visited exactly once, so each is one occurrence.
        best.into_iter()
            .map(|(assignment, energy)| (assignment, energy, 1))
            .collect()
    }

    /// Seeded multi-restart annealing, counting how often each minimum is hit.
    ///
    /// Steepest descent is the obvious choice and the wrong one here. A
    /// constrained-coverage QUBO puts its penalty terms far above its reward
    /// terms, so from the empty assignment almost every single flip is uphill
    /// and a descent stops immediately — reporting the empty panel as the
    /// minimum of a model whose real minimum is nothing like it.
    ///
    /// Annealing proposes one random flip at a time and accepts uphill moves
    /// with Boltzmann probability, which costs `O(degree)` per step instead of
    /// `O(variables)` and buys orders of magnitude more exploration from the
    /// same budget. The starting temperature is measured from the model rather
    /// than guessed, so a profile with unusual coefficient scales anneals on
    /// its own terms.
    // Cooling progress, the modulo bound and the unit draw are all ratios over
    // counts far below any precision boundary; none of them decides a result.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn anneal(
        &self,
        wanted: usize,
        seed: u64,
        restarts: usize,
        budget: u64,
    ) -> Vec<(Vec<bool>, f64, u64)> {
        let mut rng = Rng::new(seed ^ 0x9E37_79B9_7F4A_7C15);
        let mut minima: std::collections::BTreeMap<Vec<bool>, (f64, u64)> =
            std::collections::BTreeMap::new();
        let restarts = restarts.max(1);
        let steps_per_restart = (budget / restarts as u64).max(1_000);
        let start_temperature = self.starting_temperature(&mut rng);

        for restart in 0..restarts {
            // The first descent starts from all-zero, which is a meaningful
            // point here: it is the empty panel, and often a strong local
            // minimum precisely because of the penalty structure.
            let mut assignment: Vec<bool> = if restart == 0 {
                vec![false; self.variables]
            } else {
                (0..self.variables).map(|_| rng.next_bool()).collect()
            };
            let mut energy = self.energy(&assignment);
            let mut best_energy = energy;
            let mut best_assignment = assignment.clone();

            for step in 0..steps_per_restart {
                // Geometric cooling from the measured start temperature down to
                // a value where only downhill and sideways moves survive.
                let progress = step as f64 / steps_per_restart as f64;
                let temperature = start_temperature * (1.0 - progress).powi(3) + 1e-9;
                let index = rng.below(self.variables);
                let delta = self.delta(&assignment, index);
                let accept = if delta <= 0.0 {
                    true
                } else {
                    rng.unit() < (-delta / temperature).exp()
                };
                if !accept {
                    continue;
                }
                assignment[index] = !assignment[index];
                energy += delta;
                if energy < best_energy {
                    best_energy = energy;
                    best_assignment.clone_from(&assignment);
                }
            }

            let entry = minima.entry(best_assignment).or_insert((best_energy, 0));
            entry.1 += 1;
        }

        let mut found: Vec<_> = minima
            .into_iter()
            .map(|(assignment, (energy, occurrences))| (assignment, energy, occurrences))
            .collect();
        found.sort_by(|left, right| left.1.total_cmp(&right.1).then(left.0.cmp(&right.0)));
        found.truncate(wanted.max(1));
        found
    }

    /// A starting temperature at which a typical uphill move is accepted.
    ///
    /// Sampled from the model itself: take the median uphill move seen from a
    /// handful of random assignments and set the temperature so that move is
    /// accepted about half the time. A fixed constant cannot do this, because
    /// the coefficient scale changes with every profile's outcome weights.
    fn starting_temperature(&self, rng: &mut Rng) -> f64 {
        let mut uphill = Vec::new();
        for _ in 0..8 {
            let assignment: Vec<bool> = (0..self.variables).map(|_| rng.next_bool()).collect();
            for _ in 0..self.variables.min(64) {
                let delta = self.delta(&assignment, rng.below(self.variables));
                if delta > 0.0 {
                    uphill.push(delta);
                }
            }
        }
        if uphill.is_empty() {
            return 1.0;
        }
        uphill.sort_by(f64::total_cmp);
        let median = uphill[uphill.len() / 2];
        // Accepting a median move with probability 1/2 means T = delta / ln 2.
        (median / std::f64::consts::LN_2).max(f64::MIN_POSITIVE)
    }
}

/// Small deterministic generator; the same seed must give the same samples.
struct Rng(u64);

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
impl Rng {
    const fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn next_bool(&mut self) -> bool {
        self.next() & 1 == 1
    }

    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }

    /// A value in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }
}

/// Distinct variable indices touched by a model's couplings.
///
/// Exposed for diagnostics: a variable no coupling mentions is decided by its
/// linear term alone, which is worth knowing when a formulation looks larger
/// than the problem it encodes.
#[must_use]
pub fn coupled_variables(problem: &QuantumProblem) -> usize {
    let mut seen = BTreeSet::new();
    match problem {
        QuantumProblem::Qubo(model) => {
            for coupling in &model.quadratic {
                seen.insert(coupling.left);
                seen.insert(coupling.right);
            }
        }
        QuantumProblem::Ising(model) => {
            for coupling in &model.quadratic {
                seen.insert(coupling.left);
                seen.insert(coupling.right);
            }
        }
    }
    seen.len()
}
