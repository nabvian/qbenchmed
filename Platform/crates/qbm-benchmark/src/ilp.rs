//! Certified optimisation of the native integer program.
//!
//! Exhaustive enumeration proves optimality but dies at about twenty inputs,
//! which left every larger profile answered by greedy and labelled "no proof".
//! On a sixty-six input profile that cost real accuracy: greedy reported a
//! nineteen-input panel for full coverage where the true optimum is seventeen.
//!
//! This module closes that gap. The program is
//!
//! ```text
//!   maximise    sum_j w_j * y_j
//!   subject to  y_j = 1  only if outcome j is covered by the selection
//!               min_selected <= |x| <= max_selected
//!               sum_{i in x} c_i <= max_total_cost
//!               required inputs in x, excluded inputs not in x
//!               every required outcome covered
//! ```
//!
//! `y` is fully determined by `x`, so rather than relax the program and branch
//! on fractional variables the search branches directly on the input variables
//! and bounds each node by the weight of the outcomes that are still reachable
//! from it. The bound is admissible — it assumes every still-possible outcome
//! gets covered — so a completed search proves optimality exactly as a
//! branch-and-bound over an LP relaxation would. There is no LP relaxation and
//! no floating-point pivoting, which also means no external solver dependency
//! and a bit-for-bit reproducible answer.
//!
//! When the node budget runs out the search stops and says so: the result
//! carries the incumbent, the surviving bound, and `proven_optimal = false`.
//! It never reports an unproven answer as certified.

use crate::coverage::{CoverageModel, InputSet};
use crate::{BenchmarkError, BenchmarkProfile, OptimizationRequest};

/// Resource ceilings for one certified search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IlpLimits {
    /// Largest number of search nodes explored before giving up the proof.
    pub max_nodes: u64,
}

impl Default for IlpLimits {
    fn default() -> Self {
        Self {
            max_nodes: 20_000_000,
        }
    }
}

/// Outcome of one certified search.
#[derive(Debug, Clone)]
pub struct IlpSolution {
    /// Best selection found.
    pub selected: InputSet,
    /// Weight of the outcomes that selection covers.
    pub covered_weight: f64,
    /// True only when the search closed without hitting a limit.
    pub proven_optimal: bool,
    /// Search nodes explored.
    pub nodes_explored: u64,
    /// Best upper bound still outstanding when the search stopped.
    ///
    /// Equal to `covered_weight` on a proven run; strictly above it when the
    /// node budget ran out, where it is the honest remaining gap.
    pub upper_bound: f64,
}

/// The native program, compiled into index space.
#[derive(Debug, Clone)]
pub struct IlpProblem<'a> {
    model: &'a CoverageModel,
    forced_in: InputSet,
    forbidden: InputSet,
    min_selected: usize,
    max_selected: usize,
    max_total_cost: Option<f64>,
    required_outcomes: Vec<usize>,
}

impl<'a> IlpProblem<'a> {
    /// Build the program a profile and request jointly describe.
    pub fn new(
        model: &'a CoverageModel,
        profile: &BenchmarkProfile,
        request: &OptimizationRequest,
    ) -> Result<Self, BenchmarkError> {
        let mut forced_in = InputSet::empty(model.input_count);
        for id in &profile.constraints.required_inputs {
            forced_in.insert(model.index_of(id).ok_or_else(|| {
                BenchmarkError::InvalidProfile(format!("required input {id:?} is unknown"))
            })?);
        }
        let mut forbidden = InputSet::empty(model.input_count);
        for id in &profile.constraints.excluded_inputs {
            forbidden.insert(model.index_of(id).ok_or_else(|| {
                BenchmarkError::InvalidProfile(format!("excluded input {id:?} is unknown"))
            })?);
        }
        let max_selected = [
            Some(model.input_count),
            profile.constraints.max_selected,
            request.max_inputs,
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(model.input_count);

        let outcome_position = |id: &str| {
            profile
                .outcomes
                .iter()
                .position(|outcome| outcome.id == id)
                .ok_or_else(|| {
                    BenchmarkError::InvalidProfile(format!("required outcome {id:?} is unknown"))
                })
        };
        let required_outcomes = profile
            .constraints
            .required_outcomes
            .iter()
            .map(|id| outcome_position(id))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            model,
            forced_in,
            forbidden,
            min_selected: profile.constraints.min_selected,
            max_selected,
            max_total_cost: profile.constraints.max_total_cost,
            required_outcomes,
        })
    }

    /// Whether a fully decided selection satisfies every declared constraint.
    fn feasible(&self, selected: &InputSet) -> bool {
        let count = selected.len();
        if count < self.min_selected || count > self.max_selected {
            return false;
        }
        if !selected.contains_all(&self.forced_in) || selected.intersects(&self.forbidden) {
            return false;
        }
        if let Some(ceiling) = self.max_total_cost {
            if self.model.total_cost(selected) > ceiling + 1e-9 {
                return false;
            }
        }
        self.required_outcomes
            .iter()
            .all(|index| self.model.rules[*index].covered_by(selected))
    }

    /// Order free inputs by how much coverage weight they can unlock per unit
    /// cost. A good order finds strong incumbents early, which is what makes
    /// the bound prune; it never changes which answer is optimal.
    #[allow(clippy::cast_precision_loss)] // A heuristic ordering score; precision cannot change which answer is optimal.
    fn branch_order(&self) -> Vec<usize> {
        let mut potential = vec![0.0_f64; self.model.input_count];
        for (index, rule) in self.model.rules.iter().enumerate() {
            let weight = self.model.weights[index];
            for arm in &rule.arms {
                let size = arm.present_indices.len().max(1) as f64;
                for input in &arm.present_indices {
                    potential[*input] += weight / size;
                }
            }
        }
        let mut order: Vec<usize> = (0..self.model.input_count)
            .filter(|index| !self.forced_in.contains(*index) && !self.forbidden.contains(*index))
            .collect();
        order.sort_by(|left, right| {
            let left_score = potential[*left] / self.model.costs[*left].max(f64::MIN_POSITIVE);
            let right_score = potential[*right] / self.model.costs[*right].max(f64::MIN_POSITIVE);
            right_score
                .partial_cmp(&left_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(left.cmp(right))
        });
        order
    }
}

/// Certified maximum weighted coverage.
///
/// Returns the best selection together with whether the search proved it
/// optimal. `seed` is an optional starting incumbent — a greedy answer, say —
/// which only speeds up pruning and cannot change the result.
pub fn solve_max_coverage(
    problem: &IlpProblem<'_>,
    limits: IlpLimits,
    seed: Option<&InputSet>,
) -> Result<IlpSolution, BenchmarkError> {
    let model = problem.model;
    let order = problem.branch_order();

    let mut best_selection: Option<InputSet> = None;
    let mut best_weight = f64::NEG_INFINITY;
    if let Some(candidate) = seed {
        if problem.feasible(candidate) {
            best_weight = model.covered_weight(candidate);
            best_selection = Some(candidate.clone());
        }
    }

    let mut search = Search {
        problem,
        order: &order,
        limits,
        reach_prefix: reach_prefix_sums(problem),
        decided_in: problem.forced_in.clone(),
        decided_out: problem.forbidden.clone(),
        best_selection,
        best_weight,
        nodes: 0,
        exhausted: false,
    };

    // Record the root bound before descending, so an exhausted search can
    // report the gap it actually left open.
    let root_budget = problem.max_selected.saturating_sub(search.decided_in.len());
    let root_bound = search.bound(root_budget);
    search.consider_current();
    search.descend(0);

    let selected = search.best_selection.ok_or_else(|| {
        BenchmarkError::Infeasible("no selection satisfies every declared constraint".to_owned())
    })?;
    let selected = trim_redundant(problem, selected);
    let covered_weight = model.covered_weight(&selected);
    Ok(IlpSolution {
        selected,
        covered_weight,
        proven_optimal: !search.exhausted,
        nodes_explored: search.nodes,
        upper_bound: if search.exhausted {
            root_bound.max(covered_weight)
        } else {
            covered_weight
        },
    })
}

/// Drop inputs the answer does not need.
///
/// Maximum coverage is indifferent to panel size, so a certified-optimal panel
/// can carry passengers: at a ceiling of fifty the search would return
/// forty-two inputs for coverage that thirty-seven achieve. Both are optimal by
/// the stated objective, but reporting the padded one invites the reader to
/// believe those inputs are doing something.
///
/// Removing an input is only ever accepted when coverage does not fall and
/// every declared constraint still holds, so the result stays exactly as
/// optimal as the panel it came from. Removal is attempted in descending cost
/// order, which is deterministic and sheds the most expensive passenger first.
fn trim_redundant(problem: &IlpProblem<'_>, selected: InputSet) -> InputSet {
    let model = problem.model;
    let target = model.covered_weight(&selected);
    let mut trimmed = selected;
    let mut candidates = trimmed.indices();
    candidates.sort_by(|left, right| {
        model.costs[*right]
            .total_cmp(&model.costs[*left])
            .then(left.cmp(right))
    });
    for index in candidates {
        if problem.forced_in.contains(index) {
            continue;
        }
        trimmed.remove(index);
        let still_optimal =
            model.covered_weight(&trimmed) >= target - 1e-9 && problem.feasible(&trimmed);
        if !still_optimal {
            trimmed.insert(index);
        }
    }
    trimmed
}

/// Descending per-input reach weights, accumulated into prefix sums.
///
/// `reach[i]` is the total weight of every outcome input `i` could possibly
/// help cover. Any outcome newly covered by adding `r` inputs must have all of
/// one arm's missing members among those `r`, so its weight is counted in at
/// least one chosen input's reach. Summing the `r` largest reaches is therefore
/// a valid ceiling on what `r` more inputs can add — and a far tighter one than
/// "every outcome that is still theoretically possible", which at the root is
/// simply the whole profile.
fn reach_prefix_sums(problem: &IlpProblem<'_>) -> Vec<f64> {
    let model = problem.model;
    let mut reach = vec![0.0_f64; model.input_count];
    for (index, rule) in model.rules.iter().enumerate() {
        let weight = model.weights[index];
        let mut credited = InputSet::empty(model.input_count);
        for arm in &rule.arms {
            credited.union_with(&arm.present_closure);
        }
        credited.for_each(|input| reach[input] += weight);
    }
    reach.sort_by(|left, right| right.total_cmp(left));
    let mut prefix = Vec::with_capacity(reach.len() + 1);
    let mut running = 0.0;
    prefix.push(0.0);
    for value in reach {
        running += value;
        prefix.push(running);
    }
    prefix
}

/// Mutable state of one depth-first certified search.
struct Search<'a, 'p> {
    problem: &'a IlpProblem<'p>,
    order: &'a [usize],
    limits: IlpLimits,
    reach_prefix: Vec<f64>,
    decided_in: InputSet,
    decided_out: InputSet,
    best_selection: Option<InputSet>,
    best_weight: f64,
    nodes: u64,
    exhausted: bool,
}

impl Search<'_, '_> {
    /// Record the currently decided-in set if it is feasible and an improvement.
    ///
    /// Every prefix of the search is itself a complete, legal panel, so
    /// incumbents appear early rather than only at the leaves — which is what
    /// gives the bound something to prune against.
    fn consider_current(&mut self) {
        if !self.problem.feasible(&self.decided_in) {
            return;
        }
        let weight = self.problem.model.covered_weight(&self.decided_in);
        // Maximum coverage says nothing about panel size, so without a
        // tie-break the search happily returns a panel padded with inputs that
        // cover nothing. Among equally optimal panels, prefer the smaller and
        // then the cheaper — a strictly better answer to the same question.
        let improvement = match weight.total_cmp(&self.best_weight) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => self.best_selection.as_ref().is_none_or(|current| {
                let size = self.decided_in.len();
                let best_size = current.len();
                size < best_size
                    || (size == best_size
                        && self.problem.model.total_cost(&self.decided_in)
                            < self.problem.model.total_cost(current))
            }),
            std::cmp::Ordering::Less => false,
        };
        if improvement {
            self.best_weight = weight;
            self.best_selection = Some(self.decided_in.clone());
        }
    }

    /// Highest coverage weight any completion of this node could reach.
    ///
    /// Two independent ceilings, whichever is lower: the weight of what is
    /// still possible at all, and what the remaining budget can physically
    /// reach. Both over-estimate, so their minimum is still admissible.
    fn bound(&self, budget: usize) -> f64 {
        let model = self.problem.model;
        let mut secured = 0.0;
        let mut reachable = 0.0;
        for (index, rule) in model.rules.iter().enumerate() {
            let weight = model.weights[index];
            if rule.covered_by(&self.decided_in) {
                secured += weight;
            } else if rule.still_possible(&self.decided_in, &self.decided_out, budget) {
                reachable += weight;
            }
        }
        let capacity = self.reach_prefix[budget.min(self.reach_prefix.len() - 1)];
        secured + reachable.min(capacity)
    }

    /// Whether this subtree can still beat the incumbent.
    fn worth_exploring(&self, budget: usize) -> bool {
        let required_possible = self.problem.required_outcomes.iter().all(|outcome| {
            self.problem.model.rules[*outcome].still_possible(
                &self.decided_in,
                &self.decided_out,
                budget,
            )
        });
        required_possible && self.bound(budget) > self.best_weight + 1e-9
    }

    /// Branch on the input at `position`, then on everything after it.
    ///
    /// Depth is bounded by the number of free inputs, and each decision is
    /// undone on the way back out so the two branches see identical state.
    fn descend(&mut self, position: usize) {
        if self.exhausted || position >= self.order.len() {
            return;
        }
        let index = self.order[position];

        // Branch one: select this input.
        let room = self.decided_in.len() < self.problem.max_selected;
        let affordable = self.problem.max_total_cost.is_none_or(|ceiling| {
            self.problem.model.total_cost(&self.decided_in) + self.problem.model.costs[index]
                <= ceiling + 1e-9
        });
        if room && affordable {
            self.decided_in.insert(index);
            self.nodes += 1;
            if self.nodes >= self.limits.max_nodes {
                self.exhausted = true;
            } else {
                self.consider_current();
                let budget = self
                    .problem
                    .max_selected
                    .saturating_sub(self.decided_in.len());
                if self.worth_exploring(budget) {
                    self.descend(position + 1);
                }
            }
            self.decided_in.remove(index);
        }
        if self.exhausted {
            return;
        }

        // Branch two: rule this input out. Ruling an input out can *enable* an
        // outcome that an exclusionary edge would otherwise block, so this
        // branch is not a mere formality.
        self.decided_out.insert(index);
        self.nodes += 1;
        if self.nodes >= self.limits.max_nodes {
            self.exhausted = true;
        } else {
            self.consider_current();
            let budget = self
                .problem
                .max_selected
                .saturating_sub(self.decided_in.len());
            if self.worth_exploring(budget) {
                self.descend(position + 1);
            }
        }
        self.decided_out.remove(index);
    }
}

/// Certified smallest panel reaching a weighted-coverage floor.
///
/// Maximum coverage is non-decreasing in the panel ceiling — a larger ceiling
/// can always reproduce a smaller selection — so the smallest sufficient
/// ceiling is found by binary search, each probe answered by a certified
/// maximum-coverage search. The answer is certified when every probe was.
pub fn solve_minimum_panel(
    model: &CoverageModel,
    profile: &BenchmarkProfile,
    request: &OptimizationRequest,
    coverage_floor: f64,
    limits: IlpLimits,
) -> Result<IlpSolution, BenchmarkError> {
    let total_weight: f64 = model.weights.iter().sum();
    let target = coverage_floor * total_weight - 1e-9;

    let ceiling_request = |ceiling: usize| OptimizationRequest {
        max_inputs: Some(ceiling),
        ..request.clone()
    };

    let upper_problem = IlpProblem::new(model, profile, request)?;
    let highest = upper_problem.max_selected;
    let lowest = profile
        .constraints
        .min_selected
        .max(profile.constraints.required_inputs.len())
        .max(1);
    if highest < lowest {
        return Err(BenchmarkError::Infeasible(
            "no panel size satisfies both min_selected and max_selected".to_owned(),
        ));
    }

    let mut proven = true;
    let probe =
        |ceiling: usize, proven: &mut bool| -> Result<Option<IlpSolution>, BenchmarkError> {
            let scoped = ceiling_request(ceiling);
            let problem = IlpProblem::new(model, profile, &scoped)?;
            match solve_max_coverage(&problem, limits, None) {
                Ok(solution) => {
                    if !solution.proven_optimal {
                        *proven = false;
                    }
                    Ok(Some(solution))
                }
                Err(BenchmarkError::Infeasible(_)) => Ok(None),
                Err(error) => Err(error),
            }
        };

    // The largest allowed panel decides whether the floor is reachable at all.
    let Some(best_possible) = probe(highest, &mut proven)? else {
        return Err(BenchmarkError::Infeasible(
            "no selection satisfies every declared constraint".to_owned(),
        ));
    };
    if best_possible.covered_weight < target {
        return Err(BenchmarkError::Infeasible(format!(
            "the largest allowed panel covers {:.6} of {:.6} weight, below the {:.0}% floor",
            best_possible.covered_weight,
            total_weight,
            coverage_floor * 100.0
        )));
    }

    let mut low = lowest;
    let mut high = highest;
    while low < high {
        let middle = low + (high - low) / 2;
        match probe(middle, &mut proven)? {
            Some(solution) if solution.covered_weight >= target => high = middle,
            _ => low = middle + 1,
        }
    }

    // Re-solve at the settled ceiling so the returned panel is the one that
    // size actually achieves, not whichever probe happened to run last.
    let mut best = probe(low, &mut proven)?
        .filter(|s| s.covered_weight >= target)
        .ok_or_else(|| {
            BenchmarkError::Infeasible(
                "binary search settled on an infeasible panel size".to_owned(),
            )
        })?;
    best.proven_optimal = proven;
    Ok(best)
}
