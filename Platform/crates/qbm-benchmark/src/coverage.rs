//! Typed coverage semantics: when does a selected input panel cover an outcome?
//!
//! Under schema `v1` the answer was "when any selected input touches it". That
//! is only true of a rule set whose triggers are single inputs. Real triggers
//! fire on combinations, and once combinations exist three things change at
//! once: coverage stops being submodular, greedy loses its `1 - 1/e`
//! guarantee, and adding an input can *remove* coverage. This module is the
//! single place those semantics are defined, so every solver, the QUBO builder
//! and the structural metrics all answer the question the same way.
//!
//! An outcome is covered by selection `x` when
//!
//! 1. it is declared unconditional, or
//! 2. every unpathed `required` input is in `x`, **and** no unpathed
//!    `exclusionary` input is in `x`, **and** at least one arm is satisfied.
//!
//! An arm is satisfied when every input it needs present is in `x` and every
//! input it needs absent is not. Contributing members need themselves present;
//! `contextual` members additionally need their context input present;
//! `exclusionary` members need themselves absent. Unpathed `required` edges
//! form one arm of their own, so an outcome whose only edge is `required` is
//! covered as soon as that input is selected.
//!
//! With no paths and no kinds the rules collapse: conditions on `required` and
//! `exclusionary` are vacuous, every edge is its own singleton arm, and
//! coverage is exactly binary incidence again. That is what makes a `v1`
//! document safe to load unchanged.

use std::collections::BTreeMap;

use crate::{BenchmarkError, BenchmarkProfile, RelationshipKind};

/// A fixed-width set of input indices.
///
/// Coverage is evaluated once per candidate panel, and branch-and-bound
/// evaluates millions of them, so membership has to be a machine word
/// operation rather than a string lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSet {
    words: Vec<u64>,
}

impl InputSet {
    /// An empty set sized for `capacity` inputs.
    #[must_use]
    pub fn empty(capacity: usize) -> Self {
        Self {
            words: vec![0; capacity.div_ceil(64).max(1)],
        }
    }

    /// The set containing every input below `capacity`.
    #[must_use]
    pub fn full(capacity: usize) -> Self {
        let mut set = Self::empty(capacity);
        for index in 0..capacity {
            set.insert(index);
        }
        set
    }

    /// Add one input index.
    pub fn insert(&mut self, index: usize) {
        self.words[index / 64] |= 1 << (index % 64);
    }

    /// Remove one input index.
    pub fn remove(&mut self, index: usize) {
        self.words[index / 64] &= !(1 << (index % 64));
    }

    /// Whether one input index is present.
    #[must_use]
    pub fn contains(&self, index: usize) -> bool {
        self.words[index / 64] & (1 << (index % 64)) != 0
    }

    /// Number of inputs present.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// Whether no input is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// Whether every member of `other` is also present here.
    #[must_use]
    pub fn contains_all(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(&other.words)
            .all(|(mine, theirs)| mine & theirs == *theirs)
    }

    /// Whether any member is shared with `other`.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.words
            .iter()
            .zip(&other.words)
            .any(|(mine, theirs)| mine & theirs != 0)
    }

    /// Number of members of `other` that are missing here.
    #[must_use]
    pub fn missing_count(&self, other: &Self) -> usize {
        self.words
            .iter()
            .zip(&other.words)
            .map(|(mine, theirs)| (theirs & !mine).count_ones() as usize)
            .sum()
    }

    /// Present indices in ascending order.
    #[must_use]
    pub fn indices(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for (word_index, word) in self.words.iter().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                out.push(word_index * 64 + bit);
                bits &= bits - 1;
            }
        }
        out
    }
}

/// One conjunctive arm of an outcome's rule.
///
/// Satisfied when every index in `require_present` is selected and no index in
/// `require_absent` is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arm {
    /// Inputs that must all be selected, contextual contexts included.
    pub require_present: InputSet,
    /// Inputs that must all be unselected.
    pub require_absent: InputSet,
    /// `require_present` as ascending indices, for encoders that need a list.
    pub present_indices: Vec<usize>,
    /// `require_absent` as ascending indices.
    pub absent_indices: Vec<usize>,
    /// Path label this arm was built from; empty for a singleton arm.
    pub path: String,
}

impl Arm {
    /// Whether this arm is satisfied by `selected`.
    #[must_use]
    pub fn satisfied_by(&self, selected: &InputSet) -> bool {
        selected.contains_all(&self.require_present) && !selected.intersects(&self.require_absent)
    }
}

/// The compiled rule for one outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeRule {
    /// Outcome index within the profile's outcome list.
    pub outcome: usize,
    /// Covered by every selection, including the empty one.
    pub unconditional: bool,
    /// Unpathed `required` inputs, all of which must be selected.
    pub global_required: InputSet,
    /// Unpathed `exclusionary` inputs, none of which may be selected.
    pub global_excluded: InputSet,
    /// Alternative arms; the outcome needs exactly one of them.
    pub arms: Vec<Arm>,
}

impl OutcomeRule {
    /// Whether `selected` covers this outcome.
    #[must_use]
    pub fn covered_by(&self, selected: &InputSet) -> bool {
        if self.unconditional {
            return true;
        }
        if !selected.contains_all(&self.global_required) {
            return false;
        }
        if selected.intersects(&self.global_excluded) {
            return false;
        }
        self.arms.iter().any(|arm| arm.satisfied_by(selected))
    }

    /// Whether this outcome can still be covered from a partial decision.
    ///
    /// `decided_in` is already selected, `decided_out` can never be selected,
    /// and at most `budget` further inputs may still be added. Used only as a
    /// branch-and-bound bound, so it must never rule out a genuinely reachable
    /// outcome — returning `true` too often costs search time, returning
    /// `false` too often would lose the optimum.
    #[must_use]
    pub fn still_possible(
        &self,
        decided_in: &InputSet,
        decided_out: &InputSet,
        budget: usize,
    ) -> bool {
        if self.unconditional {
            return true;
        }
        if self.global_required.intersects(decided_out) {
            return false;
        }
        if self.global_excluded.intersects(decided_in) {
            return false;
        }
        let global_missing = decided_in.missing_count(&self.global_required);
        if global_missing > budget {
            return false;
        }
        self.arms.iter().any(|arm| {
            if arm.require_present.intersects(decided_out) {
                return false;
            }
            if arm.require_absent.intersects(decided_in) {
                return false;
            }
            let mut still_needed = arm.require_present.clone();
            for index in self.global_required.indices() {
                still_needed.insert(index);
            }
            decided_in.missing_count(&still_needed) <= budget
        })
    }
}

/// A profile compiled into index space, ready for repeated evaluation.
#[derive(Debug, Clone)]
pub struct CoverageModel {
    /// Number of selectable inputs.
    pub input_count: usize,
    /// Compiled rules in profile outcome order.
    pub rules: Vec<OutcomeRule>,
    /// Outcome weights in profile outcome order.
    pub weights: Vec<f64>,
    /// Input costs in profile input order.
    pub costs: Vec<f64>,
    input_index: BTreeMap<String, usize>,
}

impl CoverageModel {
    /// Compile a validated profile. Call `validate` first; this assumes
    /// references already resolve.
    #[allow(clippy::too_many_lines)] // Arm construction reads as one pass; splitting it hides the grouping rule.
    pub fn compile(profile: &BenchmarkProfile) -> Result<Self, BenchmarkError> {
        let input_index: BTreeMap<String, usize> = profile
            .inputs
            .iter()
            .enumerate()
            .map(|(index, input)| (input.id.clone(), index))
            .collect();
        let outcome_index: BTreeMap<&str, usize> = profile
            .outcomes
            .iter()
            .enumerate()
            .map(|(index, outcome)| (outcome.id.as_str(), index))
            .collect();
        let input_count = profile.inputs.len();

        let lookup = |id: &str| -> Result<usize, BenchmarkError> {
            input_index.get(id).copied().ok_or_else(|| {
                BenchmarkError::InvalidProfile(format!(
                    "relationship references unknown input {id:?}"
                ))
            })
        };

        // Arms are grouped per outcome. A non-empty path shares a group across
        // relationships; an empty path gets a group of its own, keyed by the
        // relationship's position so two singleton edges never merge.
        let mut global_required = vec![InputSet::empty(input_count); profile.outcomes.len()];
        let mut global_excluded = vec![InputSet::empty(input_count); profile.outcomes.len()];
        let mut required_arm = vec![InputSet::empty(input_count); profile.outcomes.len()];
        let mut required_arm_used = vec![false; profile.outcomes.len()];
        let mut groups: Vec<BTreeMap<String, (InputSet, InputSet, String)>> =
            vec![BTreeMap::new(); profile.outcomes.len()];

        for (position, relationship) in profile.relationships.iter().enumerate() {
            let outcome = *outcome_index
                .get(relationship.outcome_id.as_str())
                .ok_or_else(|| {
                    BenchmarkError::InvalidProfile(format!(
                        "relationship references unknown outcome {:?}",
                        relationship.outcome_id
                    ))
                })?;
            let input = lookup(&relationship.input_id)?;

            if relationship.path.is_empty() {
                match relationship.kind {
                    RelationshipKind::Exclusionary => {
                        global_excluded[outcome].insert(input);
                        continue;
                    }
                    RelationshipKind::Required => {
                        global_required[outcome].insert(input);
                        required_arm[outcome].insert(input);
                        required_arm_used[outcome] = true;
                        continue;
                    }
                    _ => {}
                }
            }

            let key = if relationship.path.is_empty() {
                format!("\u{0}singleton{position}")
            } else {
                relationship.path.clone()
            };
            let entry = groups[outcome].entry(key).or_insert_with(|| {
                (
                    InputSet::empty(input_count),
                    InputSet::empty(input_count),
                    relationship.path.clone(),
                )
            });
            if relationship.kind == RelationshipKind::Exclusionary {
                entry.1.insert(input);
            } else {
                entry.0.insert(input);
                if relationship.kind == RelationshipKind::Contextual {
                    if let Some(context) = &relationship.context_input_id {
                        entry.0.insert(lookup(context)?);
                    }
                }
            }
        }

        let unconditional: std::collections::BTreeSet<&str> = profile
            .unconditional_outcomes
            .iter()
            .map(String::as_str)
            .collect();

        let mut rules = Vec::with_capacity(profile.outcomes.len());
        for (index, outcome) in profile.outcomes.iter().enumerate() {
            let mut arms: Vec<Arm> = groups[index]
                .values()
                .map(|(present, absent, path)| Arm {
                    present_indices: present.indices(),
                    absent_indices: absent.indices(),
                    require_present: present.clone(),
                    require_absent: absent.clone(),
                    path: path.clone(),
                })
                .collect();
            if required_arm_used[index] {
                arms.push(Arm {
                    present_indices: required_arm[index].indices(),
                    absent_indices: Vec::new(),
                    require_present: required_arm[index].clone(),
                    require_absent: InputSet::empty(input_count),
                    path: String::new(),
                });
            }
            rules.push(OutcomeRule {
                outcome: index,
                unconditional: unconditional.contains(outcome.id.as_str()),
                global_required: global_required[index].clone(),
                global_excluded: global_excluded[index].clone(),
                arms,
            });
        }

        Ok(Self {
            input_count,
            rules,
            weights: profile.outcomes.iter().map(|o| o.weight).collect(),
            costs: profile.inputs.iter().map(|i| i.cost).collect(),
            input_index,
        })
    }

    /// Translate input identities into an index set, rejecting unknown ids.
    pub fn selection(&self, ids: &[String]) -> Result<InputSet, BenchmarkError> {
        let mut set = InputSet::empty(self.input_count);
        for id in ids {
            let index = self.input_index.get(id.as_str()).copied().ok_or_else(|| {
                BenchmarkError::InvalidRequest(format!("unknown input identity {id:?}"))
            })?;
            set.insert(index);
        }
        Ok(set)
    }

    /// Index of one input identity.
    #[must_use]
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.input_index.get(id).copied()
    }

    /// Covered-outcome flags for one selection, in profile outcome order.
    #[must_use]
    pub fn covered_mask(&self, selected: &InputSet) -> Vec<bool> {
        self.rules.iter().map(|r| r.covered_by(selected)).collect()
    }

    /// Total weight of the outcomes one selection covers.
    #[must_use]
    pub fn covered_weight(&self, selected: &InputSet) -> f64 {
        self.rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.covered_by(selected))
            .map(|(index, _)| self.weights[index])
            .sum()
    }

    /// Total cost of one selection.
    #[must_use]
    pub fn total_cost(&self, selected: &InputSet) -> f64 {
        selected.indices().into_iter().map(|i| self.costs[i]).sum()
    }

    /// Whether some selection covers this outcome at all.
    ///
    /// Computed structurally rather than by search: an arm is achievable when
    /// nothing it needs present is also needed absent, so the outcome is
    /// reachable when any arm is achievable alongside the global conditions.
    #[must_use]
    pub fn is_reachable(&self, outcome: usize) -> bool {
        self.is_reachable_within(outcome, &InputSet::full(self.input_count))
    }

    /// Whether some selection drawn only from `allowed` covers this outcome.
    ///
    /// Used to decide whether a required outcome survives the profile's own
    /// input exclusions, which a raw incidence lookup gets wrong as soon as an
    /// outcome needs a combination rather than a single input.
    #[must_use]
    pub fn is_reachable_within(&self, outcome: usize, allowed: &InputSet) -> bool {
        let rule = &self.rules[outcome];
        if rule.unconditional {
            return true;
        }
        if rule.global_required.intersects(&rule.global_excluded) {
            return false;
        }
        if !allowed.contains_all(&rule.global_required) {
            return false;
        }
        rule.arms.iter().any(|arm| {
            if !allowed.contains_all(&arm.require_present) {
                return false;
            }
            let mut present = arm.require_present.clone();
            for index in rule.global_required.indices() {
                present.insert(index);
            }
            let mut absent = arm.require_absent.clone();
            for index in rule.global_excluded.indices() {
                absent.insert(index);
            }
            !present.intersects(&absent)
        })
    }

    /// Inputs that can never grant coverage, with the reason they cannot.
    ///
    /// An input is *inert* when no relationship mentions it at all, and
    /// *veto-only* when every relationship that mentions it either blocks an
    /// outcome or supplies context for another input. Both are unwired for
    /// coverage, but they are unwired in different ways and an export that
    /// merges them hides the more interesting case.
    #[must_use]
    pub fn contribution_roles(&self) -> Vec<InputRole> {
        let mut roles = vec![InputRole::Inert; self.input_count];
        for rule in &self.rules {
            for index in rule.global_excluded.indices() {
                roles[index] = roles[index].max(InputRole::VetoOnly);
            }
            for index in rule.global_required.indices() {
                roles[index] = InputRole::Contributing;
            }
            for arm in &rule.arms {
                for index in &arm.present_indices {
                    roles[*index] = InputRole::Contributing;
                }
                for index in &arm.absent_indices {
                    roles[*index] = roles[*index].max(InputRole::VetoOnly);
                }
            }
        }
        roles
    }
}

/// What role an input can play in deciding coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InputRole {
    /// No relationship mentions this input.
    Inert,
    /// Every mention either blocks an outcome or supplies context.
    VetoOnly,
    /// At least one arm is advanced by selecting this input.
    Contributing,
}
