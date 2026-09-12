//! Turning a rule's conditions into the arms of an outcome.
//!
//! A rule says "when these conditions hold, emit this outcome". The conditions
//! are a *conjunction*: all of them have to hold. For panel selection that
//! means the outcome needs all of their inputs **together**, and any one of
//! them on its own buys nothing.
//!
//! The profiler used to flatten a rule into one independent edge per condition,
//! which asserts the opposite — that any single condition's input covers the
//! outcome. On a real rule set that is not a rounding error. It deletes the
//! property that makes coverage non-submodular, strands no outcomes, and lets
//! greedy look optimal on an instance where it is not.
//!
//! So conditions are read into disjunctive normal form: a set of arms, each a
//! set of inputs that must all be selected. `all_of`/`and` multiply the arms of
//! their children; `any_of`/`or` add them. Rules emitting the same outcome
//! contribute separate arms, because either rule firing covers it.
//!
//! Nothing here knows what any identifier means. The structure of the document
//! is the only evidence used, which is what lets the same extractor read a
//! haematology rule pack, an oncology one, or a project family it has never
//! seen.

use serde_json::Value;

use crate::{ProfilerLimits, SourceDocument};

/// One conjunction: every input in it must be selected together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConditionArm {
    /// Input identities and where each was observed.
    pub inputs: Vec<(String, String)>,
}

impl ConditionArm {
    fn merge(&self, other: &Self) -> Self {
        let mut inputs = self.inputs.clone();
        inputs.extend(other.inputs.iter().cloned());
        inputs.sort();
        inputs.dedup_by(|left, right| left.0 == right.0);
        Self { inputs }
    }
}

/// A disjunction of conjunctions: the outcome fires when any one arm holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConditionDnf {
    /// Alternative arms. Empty means the conditions named no usable input.
    pub arms: Vec<ConditionArm>,
    /// Whether an arm limit stopped the expansion.
    ///
    /// Reported rather than silently absorbed: a truncated rule describes less
    /// than the document said, and the reader has to be told.
    pub truncated: bool,
    /// Whether a negated branch was skipped.
    pub skipped_negation: bool,
    /// Inputs named only inside a negated branch.
    ///
    /// They create no arm, but they are real identifiers the rule set refers
    /// to, so they are still declared. They then show up as inert, which says
    /// something true: the rules mention this input, and no panel needs it.
    pub negated_inputs: Vec<(String, String)>,
}

impl ConditionDnf {
    /// The neutral element for `and`: one empty arm, vacuously satisfied.
    fn unit() -> Self {
        Self {
            arms: vec![ConditionArm::default()],
            truncated: false,
            skipped_negation: false,
            negated_inputs: Vec::new(),
        }
    }

    /// The neutral element for `or`: no arms at all, never satisfied.
    fn never() -> Self {
        Self::default()
    }

    fn is_unit(&self) -> bool {
        self.arms.len() == 1 && self.arms[0].inputs.is_empty()
    }

    /// Both sides must hold, so every pairing of their arms must hold.
    fn and(self, other: Self, limit: usize) -> Self {
        let mut negated_inputs = self.negated_inputs.clone();
        negated_inputs.extend(other.negated_inputs.iter().cloned());
        negated_inputs.sort();
        negated_inputs.dedup_by(|left, right| left.0 == right.0);
        if self.arms.is_empty() {
            return Self {
                truncated: self.truncated || other.truncated,
                skipped_negation: self.skipped_negation || other.skipped_negation,
                negated_inputs,
                ..other
            };
        }
        if other.arms.is_empty() {
            return Self {
                truncated: self.truncated || other.truncated,
                skipped_negation: self.skipped_negation || other.skipped_negation,
                negated_inputs,
                ..self
            };
        }
        let mut arms = Vec::new();
        let mut truncated = self.truncated || other.truncated;
        'outer: for left in &self.arms {
            for right in &other.arms {
                if arms.len() >= limit {
                    truncated = true;
                    break 'outer;
                }
                arms.push(left.merge(right));
            }
        }
        arms.sort_by(|left, right| left.inputs.cmp(&right.inputs));
        arms.dedup();
        Self {
            arms,
            truncated,
            skipped_negation: self.skipped_negation || other.skipped_negation,
            negated_inputs,
        }
    }

    /// Either side holding is enough, so the arms simply accumulate.
    fn or(mut self, other: Self, limit: usize) -> Self {
        let mut truncated = self.truncated || other.truncated;
        // A unit arm means "no condition at all", which would swallow every
        // alternative by being satisfied for free.
        if self.is_unit() {
            self.arms.clear();
        }
        let mut incoming = other.arms;
        if incoming.len() == 1 && incoming[0].inputs.is_empty() {
            incoming.clear();
        }
        for arm in incoming {
            if self.arms.len() >= limit {
                truncated = true;
                break;
            }
            self.arms.push(arm);
        }
        self.arms
            .sort_by(|left, right| left.inputs.cmp(&right.inputs));
        self.arms.dedup();
        let mut negated_inputs = self.negated_inputs;
        negated_inputs.extend(other.negated_inputs);
        negated_inputs.sort();
        negated_inputs.dedup_by(|left, right| left.0 == right.0);
        Self {
            arms: self.arms,
            truncated,
            skipped_negation: self.skipped_negation || other.skipped_negation,
            negated_inputs,
        }
    }
}

/// Resolves one condition node into the input identities it names.
///
/// The `bool` says whether a bare string here is a condition in its own right.
pub(crate) type ResolveInputs<'a> =
    dyn FnMut(&Value, &str, &SourceDocument, bool) -> Vec<(String, String)> + 'a;

/// Keys whose children are alternatives rather than joint requirements.
const ANY_KEYS: &[&str] = &["any", "any_of", "either", "one_of", "or"];
/// Keys whose children must all hold.
const ALL_KEYS: &[&str] = &["all", "all_of", "and", "conditions", "when"];
/// Keys whose subtree is a negative guard.
const NOT_KEYS: &[&str] = &["not", "none_of", "unless"];

/// Read a condition tree into disjunctive normal form.
///
/// `resolve` turns one condition node into the input identities it references,
/// so identifier grammar and evidence recording stay with the caller and this
/// module stays purely structural. Its `strings_are_atoms` argument says
/// whether a bare string at this position is a condition in its own right:
/// true directly under a grouping key, false under an arbitrary one, so the
/// `"IN"` in `{"operator": "IN"}` is never mistaken for an input.
///
/// `input_keys` names the keys whose values `resolve` already consumed, so the
/// walk does not descend into them and count the same identifier twice.
pub(crate) fn condition_dnf(
    value: &Value,
    pointer: &str,
    document: &SourceDocument,
    limits: ProfilerLimits,
    input_keys: &[&str],
    resolve: &mut ResolveInputs<'_>,
) -> ConditionDnf {
    walk(
        value,
        pointer,
        document,
        limits,
        input_keys,
        resolve,
        Mode::All,
        true,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    All,
    Any,
}

fn arm_limit(limits: ProfilerLimits) -> usize {
    // An arm ceiling keeps a deeply nested `any_of` from expanding into a
    // combinatorial explosion. It is deliberately generous: real rule packs sit
    // far below it, and hitting it is reported, not hidden.
    limits.max_rules.min(4_096)
}

#[allow(clippy::too_many_arguments)] // The walk's context is genuinely this wide; bundling it would only hide it.
#[allow(clippy::too_many_lines)] // One match over the node kinds; splitting it would scatter the grouping rule.
fn walk(
    value: &Value,
    pointer: &str,
    document: &SourceDocument,
    limits: ProfilerLimits,
    input_keys: &[&str],
    resolve: &mut ResolveInputs<'_>,
    mode: Mode,
    strings_are_atoms: bool,
) -> ConditionDnf {
    let limit = arm_limit(limits);
    match value {
        Value::Array(items) => {
            let mut combined = match mode {
                Mode::All => ConditionDnf::unit(),
                Mode::Any => ConditionDnf::never(),
            };
            for (index, item) in items.iter().enumerate() {
                let child_pointer = join(pointer, &index.to_string());
                let child = walk(
                    item,
                    &child_pointer,
                    document,
                    limits,
                    input_keys,
                    resolve,
                    Mode::All,
                    strings_are_atoms,
                );
                combined = match mode {
                    Mode::All => combined.and(child, limit),
                    Mode::Any => combined.or(child, limit),
                };
            }
            combined
        }
        Value::Object(object) => {
            let mut combined = ConditionDnf::unit();
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();

            // A node naming inputs directly is a leaf, whatever else it carries
            // alongside (an operator, a threshold, a comment).
            let direct = resolve(value, pointer, document, strings_are_atoms);
            if !direct.is_empty() {
                combined = ConditionDnf {
                    arms: vec![ConditionArm { inputs: direct }],
                    truncated: false,
                    skipped_negation: false,
                    negated_inputs: Vec::new(),
                };
            }

            for key in keys {
                if input_keys.contains(&key.as_str()) {
                    // Already consumed above; descending would count it twice.
                    continue;
                }
                let child_pointer = join(pointer, key);
                let child_value = &object[key];
                if NOT_KEYS.contains(&key.as_str()) {
                    // A negative guard is satisfied when its input is *absent*,
                    // so it creates no reason to select that input. Recording
                    // it as a requirement would inflate every panel. The
                    // identifiers are still collected so the profile declares
                    // them, where they will read as inert.
                    let negated = walk(
                        child_value,
                        &child_pointer,
                        document,
                        limits,
                        input_keys,
                        resolve,
                        Mode::All,
                        true,
                    );
                    for arm in negated.arms {
                        combined.negated_inputs.extend(arm.inputs);
                    }
                    combined.negated_inputs.extend(negated.negated_inputs);
                    combined.negated_inputs.sort();
                    combined
                        .negated_inputs
                        .dedup_by(|left, right| left.0 == right.0);
                    combined.skipped_negation = true;
                    continue;
                }
                let is_any = ANY_KEYS.contains(&key.as_str());
                let is_all = ALL_KEYS.contains(&key.as_str());
                let child_mode = if is_any { Mode::Any } else { Mode::All };
                // Only a grouping key introduces conditions, so only there is a
                // bare string a condition rather than a value or an operator.
                let child = walk(
                    child_value,
                    &child_pointer,
                    document,
                    limits,
                    input_keys,
                    resolve,
                    child_mode,
                    is_any || is_all,
                );
                if child.arms.is_empty() && !child.truncated && !child.skipped_negation {
                    continue;
                }
                combined = combined.and(child, limit);
            }
            combined
        }
        _ => {
            let direct = resolve(value, pointer, document, strings_are_atoms);
            if direct.is_empty() {
                ConditionDnf::never()
            } else {
                ConditionDnf {
                    arms: vec![ConditionArm { inputs: direct }],
                    truncated: false,
                    skipped_negation: false,
                    negated_inputs: Vec::new(),
                }
            }
        }
    }
}

fn join(pointer: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    format!("{pointer}/{escaped}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Stands in for the real resolver: only a `field` key names an input.
    ///
    /// Deliberately narrow, like the real one. A resolver that accepted any
    /// string would turn an operator name or a comment into an input, which is
    /// exactly the kind of guess this crate refuses to make.
    fn resolver() -> impl FnMut(&Value, &str, &SourceDocument, bool) -> Vec<(String, String)> {
        |value: &Value, pointer: &str, _document: &SourceDocument, _strings: bool| match value {
            Value::Object(object) => object
                .get("field")
                .and_then(Value::as_str)
                .map(|id| vec![(id.to_owned(), pointer.to_owned())])
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn document() -> SourceDocument {
        SourceDocument::new("rules.json", "artifact", json!({}))
    }

    fn ids(dnf: &ConditionDnf) -> Vec<Vec<String>> {
        dnf.arms
            .iter()
            .map(|arm| arm.inputs.iter().map(|(id, _)| id.clone()).collect())
            .collect()
    }

    #[test]
    fn a_list_of_conditions_is_one_arm_needing_all_of_them() {
        let value = json!([{"field": "HGB"}, {"field": "MCV"}]);
        let dnf = condition_dnf(
            &value,
            "/rules/0/conditions",
            &document(),
            ProfilerLimits::default(),
            &["field"],
            &mut resolver(),
        );
        assert_eq!(ids(&dnf), vec![vec!["HGB".to_owned(), "MCV".to_owned()]]);
    }

    #[test]
    fn any_of_produces_one_arm_per_alternative() {
        let value = json!({"any_of": [{"field": "FERRITIN"}, {"field": "TSAT"}]});
        let dnf = condition_dnf(
            &value,
            "/rules/0/conditions",
            &document(),
            ProfilerLimits::default(),
            &["field"],
            &mut resolver(),
        );
        assert_eq!(
            ids(&dnf),
            vec![vec!["FERRITIN".to_owned()], vec!["TSAT".to_owned()]]
        );
    }

    #[test]
    fn an_and_over_an_or_distributes_into_separate_arms() {
        // HGB AND (FERRITIN OR TSAT) is two ways to fire, each needing two
        // inputs. Flattening this to three independent edges would claim HGB
        // alone is enough.
        let value = json!({
            "all_of": [
                {"field": "HGB"},
                {"any_of": [{"field": "FERRITIN"}, {"field": "TSAT"}]}
            ]
        });
        let dnf = condition_dnf(
            &value,
            "/rules/0/conditions",
            &document(),
            ProfilerLimits::default(),
            &["field"],
            &mut resolver(),
        );
        assert_eq!(
            ids(&dnf),
            vec![
                vec!["FERRITIN".to_owned(), "HGB".to_owned()],
                vec!["HGB".to_owned(), "TSAT".to_owned()],
            ]
        );
    }

    #[test]
    fn a_negated_branch_creates_no_requirement() {
        let value = json!({
            "all_of": [{"field": "HGB"}, {"not": {"field": "BLASTS"}}]
        });
        let dnf = condition_dnf(
            &value,
            "/rules/0/conditions",
            &document(),
            ProfilerLimits::default(),
            &["field"],
            &mut resolver(),
        );
        assert_eq!(ids(&dnf), vec![vec!["HGB".to_owned()]]);
        assert!(
            dnf.skipped_negation,
            "skipping a negation has to be reported, not assumed away"
        );
    }

    #[test]
    fn conditions_naming_nothing_usable_produce_no_arms() {
        let value = json!({"threshold": 10, "operator": "lt"});
        let dnf = condition_dnf(
            &value,
            "/rules/0/conditions",
            &document(),
            ProfilerLimits::default(),
            &["field"],
            &mut resolver(),
        );
        assert!(dnf.arms.is_empty() || dnf.is_unit());
    }
}
