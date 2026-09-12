//! Deterministic browser-workflow artifacts built on the pure benchmark kernel.

use qbm_benchmark::{
    BenchmarkProfile, CoveragePoint, DifficultyEstimate, IsingModel, OptimizationRequest,
    OptimizationResult, ProfileDiff, QuboMetrics, QuboModel, SolverConfig, SolverKind,
    StructuralMetrics, build_qubo, coverage_at_k, diff_profiles, estimate_difficulty,
    minimum_panel, solve, structural_metrics,
};
use qbm_quantum::{
    EnergyEquivalenceConfig, EnergyEquivalenceReport, ExecutionReadiness,
    validate_energy_equivalence,
};
use serde::{Deserialize, Serialize};

pub(crate) const PROFILE_STAGE: &str = "biomedical-profile";
pub(crate) const COMPARISON_STAGE: &str = "profile-comparison";
pub(crate) const OPTIMIZATION_STAGE: &str = "classical-optimization";
pub(crate) const QUBO_STAGE: &str = "qubo-ising-validation";
pub(crate) const FULL_REPORT_STAGE: &str = "full-report";

/// A successful deterministic value or an explicit reason it was unavailable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Measured<T> {
    pub status: MeasurementStatus,
    pub value: Option<T>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MeasurementStatus {
    Complete,
    Unavailable,
}

impl<T> Measured<T> {
    fn complete(value: T) -> Self {
        Self {
            status: MeasurementStatus::Complete,
            value: Some(value),
            reason: None,
        }
    }

    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: MeasurementStatus::Unavailable,
            value: None,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MinimumPanelPoint {
    pub coverage_floor: f64,
    pub result: Measured<OptimizationResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassicalOptimizationArtifact {
    pub schema_version: String,
    pub profile_artifact_hash: String,
    pub structural: StructuralMetrics,
    pub coverage_solver: SolverKind,
    pub coverage_at_k: Vec<CoveragePoint>,
    pub minimum_panels: Vec<MinimumPanelPoint>,
    pub solver_results: Vec<Measured<OptimizationResult>>,
    pub deterministic_seed: u64,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaselineReference {
    pub run_id: String,
    pub profile_artifact_hash: String,
    pub source_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoverageDelta {
    pub k: usize,
    pub before_fraction: Option<f64>,
    pub after_fraction: Option<f64>,
    pub change: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MinimumPanelDelta {
    pub coverage_floor: f64,
    pub before_size: Option<usize>,
    pub after_size: Option<usize>,
    pub change: Option<i64>,
    pub before_reason: Option<String>,
    pub after_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuboStructureSnapshot {
    pub panel_size_ceiling: usize,
    pub metrics: QuboMetrics,
    pub difficulty: DifficultyEstimate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuboStructureDelta {
    pub before: Measured<QuboStructureSnapshot>,
    pub after: Measured<QuboStructureSnapshot>,
    pub variable_count_change: Option<i64>,
    pub coupling_count_change: Option<i64>,
    pub difficulty_score_change: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileComparisonArtifact {
    pub schema_version: String,
    pub current_profile_artifact_hash: String,
    pub baseline: Option<BaselineReference>,
    pub comparison_status: String,
    pub semantic_diff: Option<ProfileDiff>,
    pub coverage_changes: Vec<CoverageDelta>,
    pub minimum_panel_changes: Vec<MinimumPanelDelta>,
    pub qubo_change: Option<QuboStructureDelta>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QuboValidationArtifact {
    pub schema_version: String,
    pub profile_artifact_hash: String,
    pub request: OptimizationRequest,
    pub qubo: QuboModel,
    pub ising: IsingModel,
    pub metrics: QuboMetrics,
    pub difficulty: DifficultyEstimate,
    pub energy_validation: EnergyEquivalenceReport,
    pub execution: ExecutionReadiness,
    pub limitations: Vec<String>,
}

pub(crate) fn analyze_profile(
    profile: &BenchmarkProfile,
    profile_artifact_hash: &str,
) -> Result<ClassicalOptimizationArtifact, String> {
    profile.validate().map_err(|error| error.to_string())?;
    let config = SolverConfig::default();
    let solver = scalable_solver(profile);
    let panel_sizes = panel_sizes(profile.inputs.len());
    let coverage =
        coverage_at_k(profile, &panel_sizes, solver, &config).map_err(|error| error.to_string())?;
    let minimum_panels = [0.8, 0.9, 1.0]
        .into_iter()
        .map(|coverage_floor| MinimumPanelPoint {
            coverage_floor,
            result: measured(minimum_panel(profile, coverage_floor, solver, &config)),
        })
        .collect();
    let ceiling = default_panel_ceiling(profile);
    let mut kinds = vec![
        SolverKind::Greedy,
        SolverKind::SimulatedAnnealing,
        SolverKind::TabuSearch,
    ];
    if profile.inputs.len() <= config.exact_max_inputs {
        kinds.insert(0, SolverKind::Exact);
    }
    let solver_results = kinds
        .into_iter()
        .map(|kind| {
            let iterations = if matches!(
                kind,
                SolverKind::SimulatedAnnealing | SolverKind::TabuSearch
            ) {
                1_000
            } else {
                0
            };
            measured(solve(
                profile,
                &OptimizationRequest {
                    solver: kind,
                    max_inputs: Some(ceiling),
                    coverage_floor: None,
                    seed: config.default_seed,
                    iterations,
                },
                &config,
            ))
        })
        .collect();
    Ok(ClassicalOptimizationArtifact {
        schema_version: "qbm.classical-analysis/v1".to_owned(),
        profile_artifact_hash: profile_artifact_hash.to_owned(),
        structural: structural_metrics(profile).map_err(|error| error.to_string())?,
        coverage_solver: solver,
        coverage_at_k: coverage,
        minimum_panels,
        solver_results,
        deterministic_seed: config.default_seed,
        limitations: vec![
            "Exact optimality is claimed only when the exact solver completed within its declared bound.".to_owned(),
            "Greedy, simulated-annealing and tabu results are reproducible baselines, not optimality proofs.".to_owned(),
            "Results optimize only the approved profile relationships, costs, weights and constraints; they do not establish clinical validity.".to_owned(),
        ],
    })
}

pub(crate) fn compare_profile_versions(
    current: &BenchmarkProfile,
    current_hash: &str,
    baseline: Option<(&BaselineReference, &BenchmarkProfile)>,
) -> Result<ProfileComparisonArtifact, String> {
    current.validate().map_err(|error| error.to_string())?;
    let Some((baseline_reference, previous)) = baseline else {
        return Ok(ProfileComparisonArtifact {
            schema_version: "qbm.profile-comparison/v1".to_owned(),
            current_profile_artifact_hash: current_hash.to_owned(),
            baseline: None,
            comparison_status: "no_baseline".to_owned(),
            semantic_diff: None,
            coverage_changes: Vec::new(),
            minimum_panel_changes: Vec::new(),
            qubo_change: None,
            limitations: vec![
                "No earlier approved profile with the same stable profile_id was available; this run becomes the future baseline.".to_owned(),
            ],
        });
    };
    previous.validate().map_err(|error| error.to_string())?;
    let semantic_diff = diff_profiles(previous, current).map_err(|error| error.to_string())?;
    let maximum_inputs = previous.inputs.len().max(current.inputs.len());
    let sizes = requested_panel_sizes(maximum_inputs);
    let coverage_changes = sizes
        .into_iter()
        .map(|k| {
            let before = coverage_fraction(previous, k);
            let after = coverage_fraction(current, k);
            CoverageDelta {
                k,
                before_fraction: before,
                after_fraction: after,
                change: before.zip(after).map(|(left, right)| right - left),
            }
        })
        .collect();
    let config = SolverConfig::default();
    let minimum_panel_changes = [0.8, 0.9, 1.0]
        .into_iter()
        .map(|floor| {
            let before = minimum_panel(previous, floor, scalable_solver(previous), &config);
            let after = minimum_panel(current, floor, scalable_solver(current), &config);
            let before_size = before
                .as_ref()
                .ok()
                .map(|result| result.score.selected_count);
            let after_size = after
                .as_ref()
                .ok()
                .map(|result| result.score.selected_count);
            MinimumPanelDelta {
                coverage_floor: floor,
                before_size,
                after_size,
                change: before_size
                    .zip(after_size)
                    .map(|(left, right)| signed_change(right, left)),
                before_reason: before.err().map(|error| error.to_string()),
                after_reason: after.err().map(|error| error.to_string()),
            }
        })
        .collect();
    let common_ceiling = default_panel_ceiling(previous).min(default_panel_ceiling(current));
    let before_qubo = qubo_snapshot(previous, common_ceiling);
    let after_qubo = qubo_snapshot(current, common_ceiling);
    let qubo_change = Some(QuboStructureDelta {
        variable_count_change: metric_change(&before_qubo, &after_qubo, |value| {
            value.metrics.variable_count
        }),
        coupling_count_change: metric_change(&before_qubo, &after_qubo, |value| {
            value.metrics.coupling_count
        }),
        difficulty_score_change: metric_change(&before_qubo, &after_qubo, |value| {
            usize::from(value.difficulty.score)
        }),
        before: before_qubo,
        after: after_qubo,
    });
    Ok(ProfileComparisonArtifact {
        schema_version: "qbm.profile-comparison/v1".to_owned(),
        current_profile_artifact_hash: current_hash.to_owned(),
        baseline: Some(baseline_reference.clone()),
        comparison_status: "compared".to_owned(),
        semantic_diff: Some(semantic_diff),
        coverage_changes,
        minimum_panel_changes,
        qubo_change,
        limitations: vec![
            "A semantic identity is compared only because both approved profiles declare the same profile_id.".to_owned(),
            "Coverage and minimum-panel changes use exact search only up to the configured exact bound; larger profiles use the deterministic greedy baseline.".to_owned(),
            "QUBO difficulty is a structural heuristic, not a hardware runtime or quantum-advantage prediction.".to_owned(),
        ],
    })
}

pub(crate) fn validate_qubo_profile(
    profile: &BenchmarkProfile,
    profile_artifact_hash: &str,
) -> Result<QuboValidationArtifact, String> {
    profile.validate().map_err(|error| error.to_string())?;
    let request = OptimizationRequest {
        solver: SolverKind::Greedy,
        max_inputs: Some(default_panel_ceiling(profile)),
        coverage_floor: None,
        seed: SolverConfig::default().default_seed,
        iterations: 0,
    };
    let qubo = build_qubo(profile, &request).map_err(|error| error.to_string())?;
    let ising = qubo.to_ising().map_err(|error| error.to_string())?;
    let metrics = qubo.metrics();
    let difficulty = estimate_difficulty(profile, &metrics);
    let energy_validation =
        validate_energy_equivalence(&qubo, &ising, EnergyEquivalenceConfig::default())
            .map_err(|error| error.to_string())?;
    let execution = ExecutionReadiness::export_only();
    execution.validate().map_err(|error| error.to_string())?;
    Ok(QuboValidationArtifact {
        schema_version: "qbm.qubo-ising-validation/v1".to_owned(),
        profile_artifact_hash: profile_artifact_hash.to_owned(),
        request,
        qubo,
        ising,
        metrics,
        difficulty,
        energy_validation,
        execution,
        limitations: vec![
            "Validation covers the logical QUBO/Ising mapping; it does not perform hardware embedding, calibration or provider execution.".to_owned(),
            "The provider-neutral payload is exportable only after this exact stage output is approved; no credentials are stored here.".to_owned(),
            "A structural difficulty score is not a runtime, solution-quality or quantum-advantage prediction.".to_owned(),
        ],
    })
}

fn measured<T>(result: Result<T, qbm_benchmark::BenchmarkError>) -> Measured<T> {
    result.map_or_else(
        |error| Measured::unavailable(error.to_string()),
        Measured::complete,
    )
}

fn scalable_solver(profile: &BenchmarkProfile) -> SolverKind {
    if profile.inputs.len() <= SolverConfig::default().exact_max_inputs {
        SolverKind::Exact
    } else {
        SolverKind::Greedy
    }
}

fn requested_panel_sizes(input_count: usize) -> Vec<usize> {
    [5, 10, 20, 50, 100]
        .into_iter()
        .filter(|size| *size <= input_count)
        .collect()
}

fn panel_sizes(input_count: usize) -> Vec<usize> {
    let mut sizes = requested_panel_sizes(input_count);
    if sizes.last().copied() != Some(input_count) {
        sizes.push(input_count);
    }
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

fn default_panel_ceiling(profile: &BenchmarkProfile) -> usize {
    let declared = profile
        .constraints
        .max_selected
        .unwrap_or(profile.inputs.len());
    10_usize
        .min(declared)
        .max(profile.constraints.min_selected)
        .max(profile.constraints.required_inputs.len())
        .min(profile.inputs.len())
}

fn coverage_fraction(profile: &BenchmarkProfile, k: usize) -> Option<f64> {
    if k == 0 || k > profile.inputs.len() {
        return None;
    }
    coverage_at_k(
        profile,
        &[k],
        scalable_solver(profile),
        &SolverConfig::default(),
    )
    .ok()
    .and_then(|points| points.into_iter().next())
    .map(|point| point.result.score.coverage_fraction)
}

fn qubo_snapshot(profile: &BenchmarkProfile, ceiling: usize) -> Measured<QuboStructureSnapshot> {
    let result = build_qubo(
        profile,
        &OptimizationRequest {
            solver: SolverKind::Greedy,
            max_inputs: Some(ceiling),
            coverage_floor: None,
            seed: SolverConfig::default().default_seed,
            iterations: 0,
        },
    )
    .map(|qubo| {
        let metrics = qubo.metrics();
        let difficulty = estimate_difficulty(profile, &metrics);
        QuboStructureSnapshot {
            panel_size_ceiling: ceiling,
            metrics,
            difficulty,
        }
    });
    measured(result)
}

fn metric_change(
    before: &Measured<QuboStructureSnapshot>,
    after: &Measured<QuboStructureSnapshot>,
    field: impl Fn(&QuboStructureSnapshot) -> usize,
) -> Option<i64> {
    before
        .value
        .as_ref()
        .zip(after.value.as_ref())
        .map(|(left, right)| signed_change(field(right), field(left)))
}

fn signed_change(after: usize, before: usize) -> i64 {
    let after = i64::try_from(after).unwrap_or(i64::MAX);
    let before = i64::try_from(before).unwrap_or(i64::MAX);
    after.saturating_sub(before)
}

#[cfg(test)]
mod tests {
    use qbm_benchmark::{
        BENCHMARK_PROFILE_SCHEMA_VERSION, BenchmarkConstraints, BenchmarkInput, BenchmarkObjective,
        BenchmarkOutcome, BiomedicalScope, IncidenceRelationship, ProfileProvenance,
    };

    use super::*;

    fn profile(extra: bool) -> BenchmarkProfile {
        let mut inputs = vec![BenchmarkInput {
            id: "i1".to_owned(),
            label: "Input one".to_owned(),
            cost: 1.0,
            tags: vec!["test".to_owned()],
        }];
        let mut relationships = vec![IncidenceRelationship {
            input_id: "i1".to_owned(),
            outcome_id: "o1".to_owned(),
        }];
        if extra {
            inputs.push(BenchmarkInput {
                id: "i2".to_owned(),
                label: "Input two".to_owned(),
                cost: 1.0,
                tags: vec!["test".to_owned()],
            });
            relationships.push(IncidenceRelationship {
                input_id: "i2".to_owned(),
                outcome_id: "o2".to_owned(),
            });
        }
        BenchmarkProfile {
            schema_version: BENCHMARK_PROFILE_SCHEMA_VERSION.to_owned(),
            profile_id: "fixture".to_owned(),
            title: "Fixture".to_owned(),
            biomedical_scope: BiomedicalScope {
                area: "oncology".to_owned(),
                population: "synthetic".to_owned(),
                input_semantics: "test input".to_owned(),
                outcome_semantics: "test outcome".to_owned(),
            },
            inputs,
            outcomes: vec![
                BenchmarkOutcome {
                    id: "o1".to_owned(),
                    label: "Outcome one".to_owned(),
                    weight: 1.0,
                    tags: vec!["test".to_owned()],
                },
                BenchmarkOutcome {
                    id: "o2".to_owned(),
                    label: "Outcome two".to_owned(),
                    weight: 1.0,
                    tags: vec!["test".to_owned()],
                },
            ],
            relationships,
            constraints: BenchmarkConstraints {
                min_selected: 0,
                max_selected: None,
                max_total_cost: None,
                required_inputs: Vec::new(),
                excluded_inputs: Vec::new(),
                required_outcomes: Vec::new(),
            },
            objective: BenchmarkObjective::MaximizeWeightedCoverage,
            provenance: ProfileProvenance {
                generated_by: "test".to_owned(),
                source_revision: None,
                source_artifact_ids: vec!["artifact".to_owned()],
                projection_method: "fixture".to_owned(),
            },
        }
    }

    #[test]
    fn analysis_contains_coverage_minimum_panel_and_multiple_solvers() {
        let analysis = analyze_profile(&profile(true), &"a".repeat(64)).unwrap();
        assert_eq!(analysis.structural.input_count, 2);
        assert!(!analysis.coverage_at_k.is_empty());
        assert_eq!(analysis.minimum_panels.len(), 3);
        assert_eq!(analysis.solver_results.len(), 4);
    }

    #[test]
    fn comparison_reports_semantic_and_optimizer_changes() {
        let old = profile(false);
        let new = profile(true);
        let baseline = BaselineReference {
            run_id: "earlier".to_owned(),
            profile_artifact_hash: "b".repeat(64),
            source_revision: None,
        };
        let report =
            compare_profile_versions(&new, &"a".repeat(64), Some((&baseline, &old))).unwrap();
        let diff = report.semantic_diff.unwrap();
        assert_eq!(diff.inputs_added, vec!["i2"]);
        assert_eq!(diff.newly_reachable_outcomes, vec!["o2"]);
        assert!(report.qubo_change.is_some());
    }

    #[test]
    fn qubo_stage_validates_the_exact_ising_mapping() {
        let result = validate_qubo_profile(&profile(true), &"a".repeat(64)).unwrap();
        assert!(result.energy_validation.assignments_checked > 0);
        assert!(result.energy_validation.maximum_absolute_error <= f64::EPSILON);
        assert_eq!(result.metrics.input_variable_count, 2);
    }
}
