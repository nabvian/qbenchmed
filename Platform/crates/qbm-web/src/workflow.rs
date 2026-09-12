use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use qbm_app::PlatformApp;
use qbm_benchmark::{
    BenchmarkConstraints, BenchmarkObjective, BiomedicalScope, IncidenceRelationship,
    ProfileProvenance,
};
use qbm_domain::{
    Approval, AuditReport, Decision, GraphEdgeKind, GraphNodeKind, GraphPolicy, IntakePolicy,
    InventoryDisposition, RawProjectGraph, Run, RunId, RunMode, RunState, Severity, Sha256Digest,
    SourceAcquisition, SourceAcquisitionKind, SourceFormat, SourceSnapshot, StageState,
};
use qbm_profiler::{
    DiagnosticSeverity, ProjectionAssumption, ProjectionDiagnostic, ProjectionEvidence,
    ProjectionMethod, ProjectionStatus,
};
use serde::Serialize;

use crate::{
    acquisition::AcquiredSource,
    benchmark::{
        COMPARISON_STAGE, ClassicalOptimizationArtifact, FULL_REPORT_STAGE, OPTIMIZATION_STAGE,
        PROFILE_STAGE, QUBO_STAGE,
    },
    error::ApiError,
    models::{
        AuditCoverageView, CheckResultView, FormatCoverageView, GraphView, InventoryView,
        LanguageScanView, ParserCoverageView, ProjectView, ReportBundle, ReportView, RunListItem,
        SnapshotView, SourceView, StageView, WORKFLOW_STAGES, WorkflowView,
    },
    pipeline::{
        CapabilityStatus, OptimizationStageArtifact, ProfileStageArtifact, QuboStageArtifact,
        SkippedDocument, build_comparison_stage, build_optimization_stage, build_profile_stage,
        build_qubo_stage, load_artifact,
    },
};

pub(crate) fn finish_import(
    app: &PlatformApp,
    source: AcquiredSource,
    requested_name: Option<String>,
) -> Result<WorkflowView, ApiError> {
    let display_name = normalized_display_name(
        requested_name
            .as_deref()
            .unwrap_or(&source.suggested_project_name),
    )?;
    let project_id = next_project_id(app, &display_name)?;
    let kind = source.kind;
    let source_locator = source.source_locator.clone();
    let repository_id = source.repository_id;
    let requested_revision = source.requested_revision.clone();
    let resolved_revision = source.resolved_revision.clone();
    let retrieval_url = source.retrieval_url.clone();
    let provider_api_version = source.provider_api_version.clone();
    let content_sha256 = source.content_sha256.clone();
    let total_bytes = source.total_bytes;
    let managed = source.commit(&project_id)?;

    let project = app.register_project(&project_id, &display_name, &managed.source_path)?;
    let run = app.start_run(project.id.as_str(), RunMode::Governed)?;
    let inventory = app.scan_intake(run.id, &browser_intake_policy())?;
    if matches!(
        kind,
        SourceAcquisitionKind::UploadedArchive | SourceAcquisitionKind::PublicGithub
    ) && inventory.source_container_hash.as_ref() != Some(&content_sha256)
    {
        return Err(ApiError::unprocessable(
            "source_identity_mismatch",
            "The imported archive changed before it could be inventoried.",
        ));
    }

    let mut acquisition = SourceAcquisition {
        schema_version: "qbm.source-acquisition/v1".to_owned(),
        id: Sha256Digest::new("0".repeat(64)).map_err(|_| {
            ApiError::internal("The acquisition identity could not be initialized.")
        })?,
        project_id: project.id,
        kind,
        source_locator,
        repository_id,
        requested_revision,
        resolved_revision,
        retrieval_url,
        provider_api_version,
        content_sha256,
        managed_path: managed.source_path,
        total_bytes,
        created_at: chrono::Utc::now(),
    };
    acquisition.id = app.calculate_source_acquisition_id(&acquisition)?;
    let acquisition = app.save_source_acquisition(&acquisition)?;
    app.bind_run_acquisition(run.id, acquisition.id.as_str())?;
    workflow_view(app, run.id)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn workflow_view(app: &PlatformApp, run_id: RunId) -> Result<WorkflowView, ApiError> {
    let run = app.run(run_id)?;
    let project = app.project(run.project_id.as_str())?;
    let acquisition = app.run_acquisition(run_id)?;
    let inventory = app.inventories(run_id)?.into_iter().last();
    let snapshot = app.snapshots(run_id)?.into_iter().last();
    let graph = app.graphs(run_id)?.into_iter().last();
    let report = app.reports(run_id)?.into_iter().last();
    let adapter_detection = app.adapter_detections(run_id)?.into_iter().last();
    let adapter_plan = app.adapter_plans(run_id)?.into_iter().last();
    let semantic_graph = app.semantic_graphs(run_id)?.into_iter().last();
    let semantic_report = app.semantic_reports(run_id)?.into_iter().last();
    let projection_catalog = app.projection_catalogs(run_id)?.into_iter().last();
    let biomedical_profile = load_optional_stage_json(app, run_id, "biomedical-profile")?;
    let profile_comparison = load_optional_stage_json(app, run_id, "profile-comparison")?;
    let classical_optimization = load_optional_stage_json(app, run_id, "classical-optimization")?;
    let qubo_ising_validation = load_optional_stage_json(app, run_id, "qubo-ising-validation")?;
    let full_report = load_optional_stage_json(app, run_id, "full-report")?;
    let approved_profile_available =
        approved_complete_capability(app, run_id, PROFILE_STAGE, biomedical_profile.as_ref())?;
    let approved_optimization_available = approved_complete_capability(
        app,
        run_id,
        OPTIMIZATION_STAGE,
        classical_optimization.as_ref(),
    )?;
    let approved_quantum_export_available =
        approved_complete_capability(app, run_id, QUBO_STAGE, qubo_ising_validation.as_ref())?;
    let parser_coverage = snapshot
        .as_ref()
        .zip(graph.as_ref())
        .map(|(snapshot, graph)| parser_coverage(snapshot, graph));
    let audit_coverage = parser_coverage
        .as_ref()
        .zip(graph.as_ref())
        .map(|(coverage, graph)| audit_coverage(coverage, graph, report.as_ref()));
    let adapter_detection_approved = app
        .stage_output(run_id, "adapter-detection")?
        .is_some_and(|output| output.state == StageState::Approved);
    let adapters_not_applicable = adapter_detection_approved
        && adapter_detection
            .as_ref()
            .is_some_and(|report| !report.candidates.iter().any(|candidate| candidate.eligible));
    let stages = WORKFLOW_STAGES
        .iter()
        .map(|(id, label, purpose)| {
            let output = app.stage_output(run_id, id)?;
            Ok(StageView {
                id,
                label,
                purpose,
                status: if adapters_not_applicable
                    && matches!(
                        *id,
                        "adapter-plan" | "semantic-graph" | "semantic-audit" | "projection-catalog"
                    )
                    && output.is_none()
                {
                    "not_applicable".to_owned()
                } else {
                    output
                        .as_ref()
                        .map_or_else(|| "not_started".to_owned(), |value| value.state.to_string())
                },
                output,
            })
        })
        .collect::<Result<Vec<_>, qbm_app::AppError>>()?;
    app.verify_events(run_id)?;

    Ok(WorkflowView {
        schema_version: "qbm.web-workflow/v1",
        project: ProjectView {
            id: project.id,
            display_name: project.display_name,
        },
        run_id,
        run_mode: run.mode,
        run_state: run.state,
        source: SourceView::from(&acquisition),
        stages,
        inventory: inventory.as_ref().map(inventory_view),
        snapshot: snapshot.as_ref().map(|value| SnapshotView {
            id: value.id.clone(),
            policy_hash: value.policy_hash.clone(),
            files: value.artifacts.len() as u64,
            total_bytes: value.total_bytes,
        }),
        graph: graph.as_ref().map(|value| GraphView {
            id: value.id.clone(),
            policy_hash: value.policy_hash.clone(),
            nodes: value.nodes.len() as u64,
            edges: value.edges.len() as u64,
            diagnostics: value.diagnostics.clone(),
            parser_coverage: parser_coverage
                .clone()
                .expect("a persisted graph is always bound to its source snapshot"),
            language_scan: language_scan(value),
        }),
        report: report.as_ref().map(|value| ReportView {
            id: value.id.clone(),
            summary: value.summary.clone(),
            findings: value.findings.clone(),
        }),
        audit_coverage,
        adapter_detection: adapter_detection
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| ApiError::internal("Adapter detection could not be displayed."))?,
        adapter_plan: adapter_plan
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| ApiError::internal("Adapter plan could not be displayed."))?,
        semantic_graph: semantic_graph.as_ref().map(|value| {
            serde_json::json!({
                "id": value.id,
                "raw_graph_id": value.raw_graph_id,
                "adapter_plan_id": value.adapter_plan_id,
                "nodes": value.nodes.len(),
                "edges": value.edges.len(),
                "adapters": value.adapters.len(),
            })
        }),
        semantic_report: semantic_report
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| ApiError::internal("Semantic audit could not be displayed."))?,
        projection_catalog: projection_catalog.as_ref().map(|value| {
            serde_json::json!({
                "id": value.id,
                "semantic_graph_id": value.semantic_graph_id,
                "semantic_report_id": value.semantic_report_id,
                "projections": value.projections.len(),
                "available": value.projections.iter().filter(|projection| projection.available).count(),
                "unavailable": value.projections.iter().filter(|projection| !projection.available).count(),
                "items": value.projections,
            })
        }),
        biomedical_profile,
        profile_comparison,
        classical_optimization,
        qubo_ising_validation,
        full_report,
        event_chain_valid: true,
        report_download_url: (run.state == RunState::Complete)
            .then(|| format!("/api/runs/{run_id}/report")),
        profile_download_url: approved_profile_available
            .then(|| format!("/api/runs/{run_id}/profile")),
        optimization_download_url: approved_optimization_available
            .then(|| format!("/api/runs/{run_id}/optimization")),
        wiring_diagnostic_download_url: approved_profile_available
            .then(|| format!("/api/runs/{run_id}/wiring-diagnostic")),
        quantum_export_url: approved_quantum_export_available
            .then(|| format!("/api/runs/{run_id}/quantum-formulation")),
    })
}

fn approved_complete_capability(
    app: &PlatformApp,
    run_id: RunId,
    stage_id: &str,
    artifact: Option<&serde_json::Value>,
) -> Result<bool, ApiError> {
    let approved = app
        .stage_output(run_id, stage_id)?
        .is_some_and(|output| output.state == StageState::Approved);
    let complete = artifact
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str)
        == Some("complete");
    Ok(approved && complete)
}

/// Download envelope for an approved classical-analysis artifact.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApprovedOptimizationExport {
    pub schema_version: &'static str,
    pub run_id: RunId,
    pub stage_output_hash: Sha256Digest,
    pub approvals: Vec<Approval>,
    pub optimization: ClassicalOptimizationArtifact,
    pub interpretation_boundary: &'static str,
}

/// Stable totals for the profile wiring represented by the ledger.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WiringSummary {
    pub input_count: usize,
    pub wired_input_count: usize,
    pub inert_input_count: usize,
    pub outcome_count: usize,
    pub reachable_outcome_count: usize,
    pub unreachable_outcome_count: usize,
    pub relationship_count: usize,
    pub incidence_density: f64,
}

/// One input and every outcome linked to it in the approved profile.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputWiringDiagnostic {
    pub id: String,
    pub label: String,
    pub cost: f64,
    pub tags: Vec<String>,
    pub status: &'static str,
    pub linked_outcomes: Vec<String>,
    pub reason: Option<&'static str>,
    pub extracted_fields: Vec<String>,
    pub defaulted_fields: Vec<String>,
    pub evidence_refs: Vec<usize>,
    pub diagnostic_refs: Vec<usize>,
    pub assumption_refs: Vec<usize>,
}

/// One outcome and every input capable of covering it in the approved profile.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutcomeWiringDiagnostic {
    pub id: String,
    pub label: String,
    pub weight: f64,
    pub tags: Vec<String>,
    pub status: &'static str,
    pub linked_inputs: Vec<String>,
    pub reason: Option<&'static str>,
    pub extracted_fields: Vec<String>,
    pub defaulted_fields: Vec<String>,
    pub evidence_refs: Vec<usize>,
    pub diagnostic_refs: Vec<usize>,
    pub assumption_refs: Vec<usize>,
}

/// One exact incidence relationship plus references into the evidence catalog.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RelationshipWiringDiagnostic {
    pub relationship: IncidenceRelationship,
    pub evidence_refs: Vec<usize>,
    pub diagnostic_refs: Vec<usize>,
    pub assumption_refs: Vec<usize>,
}

/// Whether the adapter-specific reasoning layer participated in profile production.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SemanticLayerDiagnostic {
    pub status: &'static str,
    pub reason: Option<&'static str>,
    pub semantic_projection_id: Option<String>,
}

/// Projection provenance and review material retained alongside the wiring ledger.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectionReviewDiagnostic {
    pub method: ProjectionMethod,
    pub status: ProjectionStatus,
    pub confidence_bps: u16,
    pub extracted_fields: Vec<String>,
    pub defaulted_fields: Vec<String>,
    pub evidence_catalog: Vec<ProjectionEvidence>,
    pub diagnostics: Vec<ProjectionDiagnostic>,
    pub diagnostic_info_count: usize,
    pub diagnostic_warning_count: usize,
    pub assumptions: Vec<ProjectionAssumption>,
    pub documents_considered: usize,
    pub documents_skipped: Vec<SkippedDocument>,
    pub limitations: Vec<String>,
    pub reference_indexing: &'static str,
}

/// Evidence-linked explanation of what is and is not wired in an approved profile.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApprovedWiringDiagnosticExport {
    pub schema_version: &'static str,
    pub run_id: RunId,
    pub stage_output_hash: Sha256Digest,
    pub approvals: Vec<Approval>,
    pub profile_id: String,
    pub profile_title: String,
    pub biomedical_scope: BiomedicalScope,
    pub objective: BenchmarkObjective,
    pub constraints: BenchmarkConstraints,
    pub provenance: ProfileProvenance,
    pub summary: WiringSummary,
    pub inputs: Vec<InputWiringDiagnostic>,
    pub outcomes: Vec<OutcomeWiringDiagnostic>,
    pub relationships: Vec<RelationshipWiringDiagnostic>,
    pub semantic_layer: SemanticLayerDiagnostic,
    pub projection: ProjectionReviewDiagnostic,
    pub interpretation_boundary: &'static str,
}

pub(crate) fn approved_optimization_export(
    app: &PlatformApp,
    run_id: RunId,
) -> Result<ApprovedOptimizationExport, ApiError> {
    let output =
        required_approved_stage(app, run_id, OPTIMIZATION_STAGE, "optimization_not_ready")?;
    let artifact: OptimizationStageArtifact = load_artifact(app, output.output_hash.as_str())?;
    if artifact.status != CapabilityStatus::Complete {
        return Err(ApiError::conflict(
            "optimization_unavailable",
            "This run did not produce a valid classical optimization analysis.",
        ));
    }
    let optimization = artifact.value.ok_or_else(|| {
        ApiError::conflict(
            "optimization_unavailable",
            "This run did not produce a valid classical optimization analysis.",
        )
    })?;
    Ok(ApprovedOptimizationExport {
        schema_version: "qbm.classical-optimization-export/v1",
        run_id,
        stage_output_hash: output.output_hash,
        approvals: stage_approvals(app, run_id, OPTIMIZATION_STAGE)?,
        optimization,
        interpretation_boundary: "Classical results optimize only the approved qbm.profile. A heuristic solver result is not an optimality proof unless optimality_proven is true, and no result establishes clinical validity.",
    })
}

#[allow(clippy::too_many_lines)]
pub(crate) fn approved_wiring_diagnostic_export(
    app: &PlatformApp,
    run_id: RunId,
) -> Result<ApprovedWiringDiagnosticExport, ApiError> {
    let output = required_approved_stage(app, run_id, PROFILE_STAGE, "wiring_not_ready")?;
    let artifact: ProfileStageArtifact = load_artifact(app, output.output_hash.as_str())?;
    if artifact.status != CapabilityStatus::Complete {
        return Err(ApiError::conflict(
            "wiring_unavailable",
            "This run did not produce a valid biomedical qbm.profile to diagnose.",
        ));
    }
    let candidate = artifact.value.ok_or_else(|| {
        ApiError::conflict(
            "wiring_unavailable",
            "This run did not produce a valid biomedical qbm.profile to diagnose.",
        )
    })?;
    candidate
        .validate()
        .map_err(|_| ApiError::internal("The approved profile review record is invalid."))?;

    let profile = &candidate.profile;
    let mut outcomes_by_input = profile
        .inputs
        .iter()
        .map(|input| (input.id.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut inputs_by_outcome = profile
        .outcomes
        .iter()
        .map(|outcome| (outcome.id.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for relationship in &profile.relationships {
        outcomes_by_input
            .get_mut(&relationship.input_id)
            .expect("validated relationship references a known input")
            .insert(relationship.outcome_id.clone());
        inputs_by_outcome
            .get_mut(&relationship.outcome_id)
            .expect("validated relationship references a known outcome")
            .insert(relationship.input_id.clone());
    }

    let inputs = profile
        .inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let linked_outcomes = outcomes_by_input
                .get(&input.id)
                .expect("all profile inputs initialized in wiring map")
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let prefixes =
                entity_target_prefixes(candidate.method, "inputs", index, &input.id);
            InputWiringDiagnostic {
                id: input.id.clone(),
                label: input.label.clone(),
                cost: input.cost,
                tags: input.tags.clone(),
                status: if linked_outcomes.is_empty() {
                    "inert"
                } else {
                    "wired"
                },
                linked_outcomes,
                reason: outcomes_by_input[&input.id].is_empty().then_some(
                    "No incidence relationship in the approved qbm.profile links this input to an outcome.",
                ),
                extracted_fields: matching_fields(&candidate.extracted_fields, &prefixes),
                defaulted_fields: matching_fields(&candidate.defaulted_fields, &prefixes),
                evidence_refs: matching_evidence_refs(&candidate.evidence, &prefixes),
                diagnostic_refs: matching_diagnostic_refs(&candidate.diagnostics, &prefixes),
                assumption_refs: matching_assumption_refs(&candidate.assumptions, &prefixes),
            }
        })
        .collect::<Vec<_>>();

    let outcomes = profile
        .outcomes
        .iter()
        .enumerate()
        .map(|(index, outcome)| {
            let linked_inputs = inputs_by_outcome
                .get(&outcome.id)
                .expect("all profile outcomes initialized in wiring map")
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let prefixes =
                entity_target_prefixes(candidate.method, "outcomes", index, &outcome.id);
            OutcomeWiringDiagnostic {
                id: outcome.id.clone(),
                label: outcome.label.clone(),
                weight: outcome.weight,
                tags: outcome.tags.clone(),
                status: if linked_inputs.is_empty() {
                    "unreachable"
                } else {
                    "reachable"
                },
                linked_inputs,
                reason: inputs_by_outcome[&outcome.id].is_empty().then_some(
                    "No incidence relationship in the approved qbm.profile links an input to this outcome.",
                ),
                extracted_fields: matching_fields(&candidate.extracted_fields, &prefixes),
                defaulted_fields: matching_fields(&candidate.defaulted_fields, &prefixes),
                evidence_refs: matching_evidence_refs(&candidate.evidence, &prefixes),
                diagnostic_refs: matching_diagnostic_refs(&candidate.diagnostics, &prefixes),
                assumption_refs: matching_assumption_refs(&candidate.assumptions, &prefixes),
            }
        })
        .collect::<Vec<_>>();

    let relationships = profile
        .relationships
        .iter()
        .enumerate()
        .map(|(index, relationship)| {
            let prefixes = relationship_target_prefixes(candidate.method, index, relationship);
            RelationshipWiringDiagnostic {
                relationship: relationship.clone(),
                evidence_refs: matching_evidence_refs(&candidate.evidence, &prefixes),
                diagnostic_refs: matching_diagnostic_refs(&candidate.diagnostics, &prefixes),
                assumption_refs: matching_assumption_refs(&candidate.assumptions, &prefixes),
            }
        })
        .collect::<Vec<_>>();

    let adapter_detection = app.adapter_detections(run_id)?.into_iter().last();
    let semantic_projection_id = artifact.semantic_projection_id.clone();
    let (semantic_status, semantic_reason) = if semantic_projection_id.is_some() {
        ("complete", None)
    } else if adapter_detection.as_ref().is_some_and(|detection| {
        !detection
            .candidates
            .iter()
            .any(|candidate| candidate.eligible)
    }) {
        (
            "not_applicable",
            Some(
                "No eligible installed declarative adapter was selected; the approved profile was produced directly by the bounded biomedical profiler.",
            ),
        )
    } else {
        (
            "not_used",
            Some(
                "This approved profile was produced without an adapter-specific semantic projection.",
            ),
        )
    };

    let input_count = inputs.len();
    let wired_input_count = inputs
        .iter()
        .filter(|input| input.status == "wired")
        .count();
    let outcome_count = outcomes.len();
    let reachable_outcome_count = outcomes
        .iter()
        .filter(|outcome| outcome.status == "reachable")
        .count();
    let possible_relationships = input_count.saturating_mul(outcome_count);
    #[allow(clippy::cast_precision_loss)]
    let incidence_density = if possible_relationships == 0 {
        0.0
    } else {
        relationships.len() as f64 / possible_relationships as f64
    };
    let diagnostic_info_count = candidate
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Info)
        .count();
    let diagnostic_warning_count = candidate
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
        .count();

    Ok(ApprovedWiringDiagnosticExport {
        schema_version: "qbm.wiring-diagnostic-export/v1",
        run_id,
        stage_output_hash: output.output_hash,
        approvals: stage_approvals(app, run_id, PROFILE_STAGE)?,
        profile_id: profile.profile_id.clone(),
        profile_title: profile.title.clone(),
        biomedical_scope: profile.biomedical_scope.clone(),
        objective: profile.objective,
        constraints: profile.constraints.clone(),
        provenance: profile.provenance.clone(),
        summary: WiringSummary {
            input_count,
            wired_input_count,
            inert_input_count: input_count.saturating_sub(wired_input_count),
            outcome_count,
            reachable_outcome_count,
            unreachable_outcome_count: outcome_count.saturating_sub(reachable_outcome_count),
            relationship_count: relationships.len(),
            incidence_density,
        },
        inputs,
        outcomes,
        relationships,
        semantic_layer: SemanticLayerDiagnostic {
            status: semantic_status,
            reason: semantic_reason,
            semantic_projection_id,
        },
        projection: ProjectionReviewDiagnostic {
            method: candidate.method,
            status: candidate.status,
            confidence_bps: candidate.confidence_bps,
            extracted_fields: candidate.extracted_fields,
            defaulted_fields: candidate.defaulted_fields,
            evidence_catalog: candidate.evidence,
            diagnostics: candidate.diagnostics,
            diagnostic_info_count,
            diagnostic_warning_count,
            assumptions: candidate.assumptions,
            documents_considered: artifact.documents_considered,
            documents_skipped: artifact.documents_skipped,
            limitations: artifact.limitations,
            reference_indexing: "zero_based; *_refs index the corresponding projection catalog",
        },
        interpretation_boundary: "Inert and unreachable mean only that no incidence relationship exists in the approved qbm.profile. They are not proof that corresponding source, runtime, clinical, or reasoning logic is absent; unsupported formats, skipped documents, conservative projection, or an unavailable semantic adapter may hide relationships.",
    })
}

fn entity_target_prefixes(
    method: ProjectionMethod,
    collection: &str,
    index: usize,
    id: &str,
) -> Vec<String> {
    match method {
        ProjectionMethod::ExplicitProfile => vec![format!("/{collection}/{index}")],
        ProjectionMethod::BiomedicalHeuristic => {
            vec![format!("/{collection}/{}", escape_pointer_segment(id))]
        }
    }
}

fn relationship_target_prefixes(
    method: ProjectionMethod,
    index: usize,
    relationship: &IncidenceRelationship,
) -> Vec<String> {
    match method {
        ProjectionMethod::ExplicitProfile => vec![format!("/relationships/{index}")],
        ProjectionMethod::BiomedicalHeuristic => vec![format!(
            "/relationships/{}->{}",
            escape_pointer_segment(&relationship.input_id),
            escape_pointer_segment(&relationship.outcome_id)
        )],
    }
}

fn matching_fields(fields: &[String], prefixes: &[String]) -> Vec<String> {
    fields
        .iter()
        .filter(|field| prefixes.iter().any(|prefix| target_matches(field, prefix)))
        .cloned()
        .collect()
}

fn matching_evidence_refs(evidence: &[ProjectionEvidence], prefixes: &[String]) -> Vec<usize> {
    evidence
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            prefixes
                .iter()
                .any(|prefix| target_matches(&item.target_field, prefix))
        })
        .map(|(index, _)| index)
        .collect()
}

fn matching_diagnostic_refs(
    diagnostics: &[ProjectionDiagnostic],
    prefixes: &[String],
) -> Vec<usize> {
    diagnostics
        .iter()
        .enumerate()
        .filter(|(_, diagnostic)| {
            diagnostic.evidence.iter().any(|evidence| {
                prefixes
                    .iter()
                    .any(|prefix| target_matches(&evidence.target_field, prefix))
            })
        })
        .map(|(index, _)| index)
        .collect()
}

fn matching_assumption_refs(
    assumptions: &[ProjectionAssumption],
    prefixes: &[String],
) -> Vec<usize> {
    assumptions
        .iter()
        .enumerate()
        .filter(|(_, assumption)| {
            prefixes
                .iter()
                .any(|prefix| target_matches(&assumption.target_field, prefix))
        })
        .map(|(index, _)| index)
        .collect()
}

fn target_matches(target: &str, prefix: &str) -> bool {
    target == prefix
        || target
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn escape_pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

pub(crate) fn approved_profile_export(
    app: &PlatformApp,
    run_id: RunId,
) -> Result<qbm_benchmark::BenchmarkProfile, ApiError> {
    let output = required_approved_stage(app, run_id, PROFILE_STAGE, "profile_not_ready")?;
    let artifact: ProfileStageArtifact = load_artifact(app, output.output_hash.as_str())?;
    if artifact.status != CapabilityStatus::Complete {
        return Err(ApiError::conflict(
            "profile_unavailable",
            "This run did not produce a valid biomedical qbm.profile.",
        ));
    }
    artifact
        .value
        .map(|candidate| candidate.profile)
        .ok_or_else(|| {
            ApiError::conflict(
                "profile_unavailable",
                "This run did not produce a valid biomedical qbm.profile.",
            )
        })
}

pub(crate) fn approved_quantum_export(
    app: &PlatformApp,
    run_id: RunId,
) -> Result<serde_json::Value, ApiError> {
    let output = required_approved_stage(app, run_id, QUBO_STAGE, "quantum_formulation_not_ready")?;
    let artifact: QuboStageArtifact = load_artifact(app, output.output_hash.as_str())?;
    if artifact.status != CapabilityStatus::Complete {
        return Err(ApiError::conflict(
            "quantum_formulation_unavailable",
            "This run did not produce a valid QUBO/Ising formulation.",
        ));
    }
    let value = artifact.value.ok_or_else(|| {
        ApiError::conflict(
            "quantum_formulation_unavailable",
            "This run did not produce a valid QUBO/Ising formulation.",
        )
    })?;
    let approvals = stage_approvals(app, run_id, QUBO_STAGE)?;
    Ok(serde_json::json!({
        "schema_version": "qbm.quantum-formulation-export/v1",
        "run_id": run_id,
        "stage_output_hash": output.output_hash,
        "approvals": approvals,
        "formulation": value,
        "execution_boundary": "Validated logical formulation only. A separately configured executor must create and approve an exact provider request before submission.",
        "interpretation": qbm_quantum::NO_QUANTUM_ADVANTAGE_CLAIM,
    }))
}

fn stage_approvals(
    app: &PlatformApp,
    run_id: RunId,
    stage_id: &str,
) -> Result<Vec<Approval>, ApiError> {
    Ok(app
        .approvals(run_id)?
        .into_iter()
        .filter(|approval| approval.stage_id.as_str() == stage_id)
        .collect())
}

fn required_approved_stage(
    app: &PlatformApp,
    run_id: RunId,
    stage_id: &str,
    code: &'static str,
) -> Result<qbm_domain::StageOutput, ApiError> {
    let output = app
        .stage_output(run_id, stage_id)?
        .ok_or_else(|| ApiError::conflict(code, "The requested approved artifact is not ready."))?;
    if output.state != StageState::Approved {
        return Err(ApiError::conflict(
            code,
            "The requested artifact must be approved before it can be exported.",
        ));
    }
    Ok(output)
}

fn language_scan(graph: &RawProjectGraph) -> LanguageScanView {
    let mut view = LanguageScanView {
        schema_version: "qbm.language-scan-summary/v1",
        rust_modules: 0,
        rust_items: 0,
        rust_imports: 0,
        python_modules: 0,
        python_classes: 0,
        python_functions: 0,
        python_tests: 0,
        python_calls: 0,
        python_imports: 0,
        dependency_count: 0,
        dependency_samples: Vec::new(),
        limitations: vec![
            "Rust and Python are parsed statically; imported project code, build scripts, macros, decorators and tests are not executed.".to_owned(),
            "Import names are observed dependencies, not proof that a package is installed, reachable at runtime, or compatible.".to_owned(),
        ],
    };
    let formats = graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.format))
        .collect::<BTreeMap<_, _>>();
    for node in &graph.nodes {
        match node.kind {
            GraphNodeKind::RustModule => view.rust_modules += 1,
            GraphNodeKind::RustItem => view.rust_items += 1,
            GraphNodeKind::PythonModule => view.python_modules += 1,
            GraphNodeKind::PythonClass => view.python_classes += 1,
            GraphNodeKind::PythonFunction => view.python_functions += 1,
            GraphNodeKind::PythonTest => view.python_tests += 1,
            GraphNodeKind::PythonCall => view.python_calls += 1,
            GraphNodeKind::Project
            | GraphNodeKind::File
            | GraphNodeKind::Object
            | GraphNodeKind::Array
            | GraphNodeKind::Scalar => {}
        }
    }
    let mut dependencies = BTreeSet::new();
    for edge in &graph.edges {
        if edge.kind != GraphEdgeKind::Imports {
            continue;
        }
        match formats.get(&edge.from) {
            Some(SourceFormat::Rust) => view.rust_imports += 1,
            Some(SourceFormat::Python) => view.python_imports += 1,
            _ => {}
        }
        if let Some(reference) = &edge.reference {
            dependencies.insert(reference.clone());
        }
    }
    view.dependency_count = dependencies.len() as u64;
    view.dependency_samples = dependencies.into_iter().take(200).collect();
    view
}

fn load_optional_stage_json(
    app: &PlatformApp,
    run_id: RunId,
    stage_id: &str,
) -> Result<Option<serde_json::Value>, ApiError> {
    const MAXIMUM_STAGE_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
    app.stage_output(run_id, stage_id)?
        .map(|output| {
            app.load_json_artifact_bounded(
                output.output_hash.as_str(),
                MAXIMUM_STAGE_ARTIFACT_BYTES,
            )
            .map_err(ApiError::from)
        })
        .transpose()
}

pub(crate) fn recent_runs(app: &PlatformApp) -> Result<Vec<RunListItem>, ApiError> {
    let projects = app
        .projects()?
        .into_iter()
        .map(|project| (project.id.clone(), project.display_name))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut output = app
        .runs(None)?
        .into_iter()
        .rev()
        .filter_map(|run| {
            let source = app.run_acquisition(run.id).ok()?;
            let project_name = projects.get(&run.project_id)?.clone();
            Some(RunListItem {
                run_id: run.id,
                project_id: run.project_id,
                project_name,
                source_kind: Some(source.kind),
                state: run.state,
                updated_at: run.updated_at,
            })
        })
        .take(10)
        .collect::<Vec<_>>();
    output.sort_by_key(|value| std::cmp::Reverse(value.updated_at));
    Ok(output)
}

pub(crate) fn decide_and_advance(
    app: &PlatformApp,
    run_id: RunId,
    stage_id: &str,
    expected_output_hash: &str,
    decision: Decision,
    actor_id: &str,
    reason: Option<String>,
) -> Result<WorkflowView, ApiError> {
    if stage_id == "inventory" && decision == Decision::Approve {
        let inventory = app
            .inventories(run_id)?
            .into_iter()
            .find(|inventory| inventory.inventory_hash.as_str() == expected_output_hash)
            .ok_or_else(|| {
                ApiError::conflict(
                    "stale_output",
                    "That inventory is no longer the current review result.",
                )
            })?;
        if inventory.blocked_entries > 0 {
            return Err(ApiError::unprocessable(
                "blocked_inventory",
                format!(
                    "{} unsafe source entries must be corrected before evidence can be frozen.",
                    inventory.blocked_entries
                ),
            ));
        }
    }
    app.decide_stage(
        run_id,
        stage_id,
        expected_output_hash,
        decision,
        actor_id,
        reason,
    )?;
    if decision == Decision::Approve {
        advance_run(app, run_id)?;
    }
    workflow_view(app, run_id)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn advance_run(app: &PlatformApp, run_id: RunId) -> Result<Run, ApiError> {
    let run = app.run(run_id)?;
    if run.state == RunState::Complete {
        return Ok(run);
    }
    if run.state != RunState::Running {
        return Err(ApiError::conflict(
            "run_not_advanceable",
            format!("The run is {} and cannot advance.", run.state),
        ));
    }

    if let Some(output) = app.stage_output(run_id, FULL_REPORT_STAGE)? {
        if output.state == StageState::Approved {
            return Ok(app.complete_derived_run(
                run_id,
                FULL_REPORT_STAGE,
                output.output_hash.as_str(),
            )?);
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, QUBO_STAGE)? {
        if output.state == StageState::Approved {
            let report = full_report_artifact(app, run_id)?;
            app.record_derived_stage_output(
                run_id,
                QUBO_STAGE,
                output.output_hash.as_str(),
                FULL_REPORT_STAGE,
                &report,
            )?;
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, OPTIMIZATION_STAGE)? {
        if output.state == StageState::Approved {
            let profile = required_stage_output(app, run_id, PROFILE_STAGE)?;
            let artifact = build_qubo_stage(app, profile.output_hash.as_str())?;
            app.record_derived_stage_output(
                run_id,
                OPTIMIZATION_STAGE,
                output.output_hash.as_str(),
                QUBO_STAGE,
                &artifact,
            )?;
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, COMPARISON_STAGE)? {
        if output.state == StageState::Approved {
            let profile = required_stage_output(app, run_id, PROFILE_STAGE)?;
            let artifact = build_optimization_stage(app, profile.output_hash.as_str())?;
            app.record_derived_stage_output(
                run_id,
                COMPARISON_STAGE,
                output.output_hash.as_str(),
                OPTIMIZATION_STAGE,
                &artifact,
            )?;
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, PROFILE_STAGE)? {
        if output.state == StageState::Approved {
            let artifact = build_comparison_stage(app, run_id, output.output_hash.as_str())?;
            app.record_derived_stage_output(
                run_id,
                PROFILE_STAGE,
                output.output_hash.as_str(),
                COMPARISON_STAGE,
                &artifact,
            )?;
        }
        return Ok(app.run(run_id)?);
    }

    if let Some(output) = app.stage_output(run_id, "projection-catalog")? {
        if output.state == StageState::Approved {
            produce_profile_stage(
                app,
                run_id,
                "projection-catalog",
                output.output_hash.as_str(),
                Some(output.output_hash.to_string()),
            )?;
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, "semantic-audit")? {
        if output.state == StageState::Approved {
            let graph = required_stage_output(app, run_id, "semantic-graph")?;
            app.build_projection_catalog(
                run_id,
                graph.output_hash.as_str(),
                output.output_hash.as_str(),
            )?;
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, "semantic-graph")? {
        if output.state == StageState::Approved {
            app.run_semantic_audit(run_id, output.output_hash.as_str())?;
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, "adapter-plan")? {
        if output.state == StageState::Approved {
            app.build_semantic_graph(run_id, output.output_hash.as_str())?;
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, "adapter-detection")? {
        if output.state == StageState::Approved {
            let detection = app.adapter_detection(output.output_hash.as_str())?;
            let mut selected_adapter_ids = BTreeSet::new();
            let selected = detection
                .candidates
                .iter()
                .filter(|candidate| candidate.eligible)
                .filter(|candidate| selected_adapter_ids.insert(candidate.adapter_id.to_string()))
                .map(|candidate| candidate.package_hash.to_string())
                .collect::<Vec<_>>();
            if selected.is_empty() {
                produce_profile_stage(
                    app,
                    run_id,
                    "adapter-detection",
                    output.output_hash.as_str(),
                    None,
                )?;
            } else {
                app.create_adapter_plan(run_id, output.output_hash.as_str(), &selected)?;
            }
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, "generic-audit")? {
        if output.state == StageState::Approved {
            let graph = required_stage_output(app, run_id, "raw-graph")?;
            app.detect_adapters(run_id, graph.output_hash.as_str())?;
        }
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, "raw-graph")?
        && output.state == StageState::Approved
    {
        app.run_generic_audit(run_id, output.output_hash.as_str())?;
        return Ok(app.run(run_id)?);
    }
    if app.stage_output(run_id, "raw-graph")?.is_some() {
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, "source-snapshot")?
        && output.state == StageState::Approved
    {
        app.build_graph(run_id, output.output_hash.as_str(), &GraphPolicy::default())?;
        return Ok(app.run(run_id)?);
    }
    if app.stage_output(run_id, "source-snapshot")?.is_some() {
        return Ok(app.run(run_id)?);
    }
    if let Some(output) = app.stage_output(run_id, "inventory")?
        && output.state == StageState::Approved
    {
        let inventory = app
            .inventories(run_id)?
            .into_iter()
            .find(|inventory| inventory.inventory_hash == output.output_hash)
            .ok_or_else(|| {
                ApiError::conflict(
                    "inventory_missing",
                    "The approved inventory record could not be found.",
                )
            })?;
        app.create_snapshot(run_id, inventory.id)?;
    }
    Ok(app.run(run_id)?)
}

fn required_stage_output(
    app: &PlatformApp,
    run_id: RunId,
    stage_id: &str,
) -> Result<qbm_domain::StageOutput, ApiError> {
    app.stage_output(run_id, stage_id)?.ok_or_else(|| {
        ApiError::conflict(
            "predecessor_missing",
            format!("The required {stage_id} result is missing from this run."),
        )
    })
}

fn produce_profile_stage(
    app: &PlatformApp,
    run_id: RunId,
    predecessor_stage: &str,
    predecessor_hash: &str,
    semantic_projection_id: Option<String>,
) -> Result<(), ApiError> {
    let run = app.run(run_id)?;
    let project = app.project(run.project_id.as_str())?;
    let acquisition = app.run_acquisition(run_id)?;
    let snapshot = app.snapshots(run_id)?.into_iter().last().ok_or_else(|| {
        ApiError::conflict(
            "snapshot_missing",
            "The immutable source snapshot is missing from this run.",
        )
    })?;
    let artifact = build_profile_stage(
        app,
        &snapshot,
        &project,
        &acquisition,
        semantic_projection_id,
    );
    app.record_derived_stage_output(
        run_id,
        predecessor_stage,
        predecessor_hash,
        PROFILE_STAGE,
        &artifact,
    )?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn full_report_artifact(app: &PlatformApp, run_id: RunId) -> Result<serde_json::Value, ApiError> {
    let workflow = workflow_view(app, run_id)?;
    let semantic_complete = workflow.semantic_graph.is_some()
        && workflow.semantic_report.is_some()
        && workflow.projection_catalog.is_some();
    let semantic_reason = (!semantic_complete).then_some(
        "No eligible installed declarative adapter was selected; the adapter-specific semantic graph, policies and projection catalog were not run.",
    );
    let mut sections = vec![serde_json::json!({
        "name": "audit",
        "status": "complete",
        "artifact_hash": workflow.report.as_ref().map(|report| report.id.to_string()),
        "reason": null,
    })];
    sections.push(serde_json::json!({
        "name": "semantic",
        "status": if semantic_complete { "complete" } else { "skipped" },
        "artifact_hash": workflow.projection_catalog.as_ref().and_then(|catalog| catalog.get("id")).and_then(serde_json::Value::as_str),
        "reason": semantic_reason,
    }));
    sections.push(capability_section(
        "profile",
        workflow.biomedical_profile.as_ref(),
        PROFILE_STAGE,
        &workflow.stages,
    ));
    sections.push(capability_section(
        "comparison",
        workflow.profile_comparison.as_ref(),
        COMPARISON_STAGE,
        &workflow.stages,
    ));
    sections.push(capability_section(
        "optimization",
        workflow.classical_optimization.as_ref(),
        OPTIMIZATION_STAGE,
        &workflow.stages,
    ));
    sections.push(capability_section(
        "quantum_formulation",
        workflow.qubo_ising_validation.as_ref(),
        QUBO_STAGE,
        &workflow.stages,
    ));
    sections.push(serde_json::json!({
        "name": "reproducibility",
        "status": "complete",
        "artifact_hash": null,
        "reason": null,
    }));
    let stage_outputs = workflow
        .stages
        .iter()
        .filter_map(|stage| {
            stage.output.as_ref().map(|output| {
                serde_json::json!({
                    "stage_id": stage.id,
                    "status": stage.status,
                    "output_hash": output.output_hash,
                })
            })
        })
        .collect::<Vec<_>>();
    let approvals = app.approvals(run_id)?;
    let events = app.events(run_id)?;
    Ok(serde_json::json!({
        "schema_version": "qbm.full-report/v1",
        "framework_scope": "A Modular Open-Source Quantum-Classical Benchmarking Framework for Biomedical Input Optimization",
        "project": workflow.project,
        "source": workflow.source,
        "sections": sections,
        "audit": {
            "generic_report": workflow.report,
            "coverage_ledger": workflow.audit_coverage,
            "parser_coverage": workflow.graph.as_ref().map(|graph| &graph.parser_coverage),
            "language_scan": workflow.graph.as_ref().map(|graph| &graph.language_scan),
        },
        "semantic": {
            "status": if semantic_complete { "complete" } else { "skipped" },
            "reason": semantic_reason,
            "adapter_detection": workflow.adapter_detection,
            "adapter_plan": workflow.adapter_plan,
            "semantic_graph": workflow.semantic_graph,
            "semantic_audit": workflow.semantic_report,
            "projection_catalog": workflow.projection_catalog,
        },
        "profile": workflow.biomedical_profile,
        "comparison": workflow.profile_comparison,
        "optimization": workflow.classical_optimization,
        "quantum": workflow.qubo_ising_validation,
        "reproducibility": {
            "platform_version": env!("CARGO_PKG_VERSION"),
            "run_id": run_id,
            "source_sha256": workflow.source.content_sha256,
            "source_revision": workflow.source.resolved_revision,
            "stage_outputs": stage_outputs,
            "approvals": approvals,
            "events": events,
            "event_chain_valid_before_final_report": workflow.event_chain_valid,
            "determinism": "Canonical JSON identities, immutable source artifacts and explicit solver seeds are retained for replay.",
        },
        "limitations": [
            "Static source analysis does not execute imported project code, build scripts, tests, models or external APIs.",
            "Heuristic biomedical projection is a review candidate, not clinical validation; explicit qbm.profile export is authoritative when supplied.",
            "Classical heuristic results do not prove optimality unless the result explicitly says optimality_proven=true.",
            "QUBO/Ising validation is logical-model validation. External quantum execution is optional, separately approved and not performed by this default local workflow.",
            "No Q-BenchMed result establishes clinical efficacy, safety, regulatory compliance or quantum advantage."
        ]
    }))
}

fn capability_section(
    name: &str,
    artifact: Option<&serde_json::Value>,
    stage_id: &str,
    stages: &[StageView],
) -> serde_json::Value {
    let status = artifact
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("skipped");
    let reason = artifact
        .and_then(|value| value.get("skip_reason"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let artifact_hash = stages
        .iter()
        .find(|stage| stage.id == stage_id)
        .and_then(|stage| stage.output.as_ref())
        .map(|output| output.output_hash.to_string());
    serde_json::json!({
        "name": name,
        "status": status,
        "artifact_hash": artifact_hash,
        "reason": reason,
    })
}

pub(crate) fn report_bundle(app: &PlatformApp, run_id: RunId) -> Result<ReportBundle, ApiError> {
    let workflow = workflow_view(app, run_id)?;
    if workflow.run_state != RunState::Complete {
        return Err(ApiError::conflict(
            "report_not_ready",
            "The report is available after the final exact-result approval.",
        ));
    }
    Ok(ReportBundle {
        schema_version: "qbm.audit-bundle/v2",
        project: workflow.project,
        source: workflow.source,
        run_id,
        run_state: workflow.run_state,
        approvals: app.approvals(run_id)?,
        inventory: workflow
            .inventory
            .ok_or_else(|| missing_output("inventory"))?,
        snapshot: workflow
            .snapshot
            .ok_or_else(|| missing_output("source snapshot"))?,
        graph: workflow.graph.ok_or_else(|| missing_output("raw graph"))?,
        generic_report: workflow
            .report
            .ok_or_else(|| missing_output("generic report"))?,
        audit_coverage: workflow
            .audit_coverage
            .ok_or_else(|| missing_output("audit coverage ledger"))?,
        full_report: workflow.full_report,
        events: app.events(run_id)?,
        event_chain_valid: workflow.event_chain_valid,
    })
}

#[derive(Debug, Default)]
struct FormatStats {
    files: u64,
    bytes: u64,
    parsed_files: u64,
    parsed_bytes: u64,
    eligible: bool,
}

fn parser_coverage(snapshot: &SourceSnapshot, graph: &RawProjectGraph) -> ParserCoverageView {
    let parsed_paths = graph
        .nodes
        .iter()
        .filter(|node| !matches!(node.kind, GraphNodeKind::Project | GraphNodeKind::File))
        .filter_map(|node| node.evidence.relative_path.clone())
        .collect::<BTreeSet<_>>();
    let mut formats = BTreeMap::<String, FormatStats>::new();
    for artifact in &snapshot.artifacts {
        let (format, eligible) = source_format_label(&artifact.relative_path);
        let stats = formats.entry(format).or_default();
        stats.files = stats.files.saturating_add(1);
        stats.bytes = stats.bytes.saturating_add(artifact.size_bytes);
        stats.eligible = eligible;
        if eligible && parsed_paths.contains(&artifact.relative_path) {
            stats.parsed_files = stats.parsed_files.saturating_add(1);
            stats.parsed_bytes = stats.parsed_bytes.saturating_add(artifact.size_bytes);
        }
    }
    let total_files = snapshot.artifacts.len() as u64;
    let total_bytes = snapshot.total_bytes;
    let parser_eligible_files = formats
        .values()
        .filter(|stats| stats.eligible)
        .map(|stats| stats.files)
        .sum();
    let parser_eligible_bytes = formats
        .values()
        .filter(|stats| stats.eligible)
        .map(|stats| stats.bytes)
        .sum();
    let parsed_files = formats.values().map(|stats| stats.parsed_files).sum();
    let parsed_bytes = formats.values().map(|stats| stats.parsed_bytes).sum();
    let unsupported_files = total_files.saturating_sub(parser_eligible_files);
    let unsupported_bytes = total_bytes.saturating_sub(parser_eligible_bytes);
    let formats = formats
        .into_iter()
        .map(|(format, stats)| FormatCoverageView {
            format,
            files: stats.files,
            bytes: stats.bytes,
            parsed_files: stats.parsed_files,
            parsed_bytes: stats.parsed_bytes,
            status: if !stats.eligible {
                "unsupported"
            } else if stats.parsed_files == stats.files {
                "parsed"
            } else if stats.parsed_files == 0 {
                "not_parsed"
            } else {
                "partially_parsed"
            },
        })
        .collect();
    ParserCoverageView {
        schema_version: "qbm.parser-coverage/v1",
        total_files,
        total_bytes,
        parser_eligible_files,
        parser_eligible_bytes,
        parsed_files,
        parsed_bytes,
        file_coverage_basis_points: basis_points(parsed_files, total_files),
        byte_coverage_basis_points: basis_points(parsed_bytes, total_bytes),
        formats,
        unsupported_files,
        unsupported_bytes,
    }
}

fn source_format_label(path: &str) -> (String, bool) {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("no_extension")
        .to_ascii_lowercase();
    match extension.as_str() {
        "json" => ("json".to_owned(), true),
        "yaml" | "yml" => ("yaml".to_owned(), true),
        "toml" => ("toml".to_owned(), true),
        "rs" => ("rust".to_owned(), true),
        "py" => ("python".to_owned(), true),
        _ => (format!("unsupported:{extension}"), false),
    }
}

fn basis_points(numerator: u64, denominator: u64) -> u16 {
    if denominator == 0 {
        return 10_000;
    }
    let value = (u128::from(numerator) * 10_000) / u128::from(denominator);
    u16::try_from(value.min(10_000)).unwrap_or(10_000)
}

#[allow(clippy::too_many_lines)]
fn audit_coverage(
    coverage: &ParserCoverageView,
    graph: &RawProjectGraph,
    report: Option<&AuditReport>,
) -> AuditCoverageView {
    let mut checks = vec![CheckResultView {
        check_id: "source.parser-coverage/v1",
        family: "source",
        status: if coverage.parsed_files == coverage.total_files {
            "pass"
        } else {
            "partial"
        },
        findings: coverage.total_files.saturating_sub(coverage.parsed_files),
        explanation: format!(
            "{} of {} files and {} of {} bytes received structured parsing.",
            coverage.parsed_files,
            coverage.total_files,
            coverage.parsed_bytes,
            coverage.total_bytes
        ),
        skip_reason: (coverage.unsupported_files > 0).then(|| {
            format!(
                "{} files ({} bytes) use formats without an active parser.",
                coverage.unsupported_files, coverage.unsupported_bytes
            )
        }),
    }];
    let diagnostic_status = graph
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.severity)
        .max()
        .map_or("pass", severity_status);
    checks.push(CheckResultView {
        check_id: "source.parse-integrity/v1",
        family: "source",
        status: diagnostic_status,
        findings: graph.diagnostics.len() as u64,
        explanation: "Supported source files were parsed without executing imported project code."
            .to_owned(),
        skip_reason: None,
    });
    for (check_id, code, explanation) in [
        (
            "generic.duplicate-identifier/v1",
            "generic.duplicate_identifier",
            "Checks scanner-recognized definitions for repeated identifiers.",
        ),
        (
            "generic.duplicate-content/v1",
            "generic.duplicate_file_content",
            "Checks frozen files for byte-identical content.",
        ),
        (
            "generic.unresolved-reference/v1",
            "generic.unresolved_reference",
            "Checks scanner-recognized references for unresolved targets.",
        ),
        (
            "generic.reference-cycle/v1",
            "generic.reference_cycle",
            "Checks recognized reference relationships for cycles.",
        ),
        (
            "generic.unreachable-definition/v1",
            "generic.unreachable_definition",
            "Checks recognized definitions for structural reachability.",
        ),
        (
            "generic.provenance/v1",
            "generic.missing_provenance",
            "Checks graph facts for immutable source evidence.",
        ),
    ] {
        checks.push(report_check(report, check_id, code, explanation));
    }
    for (check_id, family, reason) in [
        (
            "semantic.adapter-projection/v1",
            "semantic",
            "No approved biomedical semantic adapter/profile had run at this source-audit checkpoint.",
        ),
        (
            "behavior.approved-fixtures/v1",
            "behavior",
            "Imported project code and tests are not executed by the generic source audit.",
        ),
        (
            "diff.profile-version/v1",
            "diff",
            "No approved benchmark profile baseline had been selected at this checkpoint.",
        ),
        (
            "optimization.input-coverage/v1",
            "optimization",
            "No approved input/outcome profile and objective were available at this checkpoint.",
        ),
        (
            "quantum.qubo-ising/v1",
            "quantum",
            "QUBO/Ising validation occurs later and requires an approved benchmark profile.",
        ),
        (
            "quantum.external-execution/v1",
            "quantum",
            "External quantum execution is optional and was not requested or approved.",
        ),
    ] {
        checks.push(CheckResultView {
            check_id,
            family,
            status: "skipped",
            findings: 0,
            explanation: "This capability did not run.".to_owned(),
            skip_reason: Some(reason.to_owned()),
        });
    }
    let mut limitations = vec![
        "Completion means the generic source-audit checkpoint finished; later report sections record whether biomedical profiling, comparison, optimization, and QUBO/Ising validation completed.".to_owned(),
        "A Git repository supplies evidence, but does not by itself define biomedical inputs, outcomes, costs, constraints, or an optimization objective.".to_owned(),
        "The event chain is internally hash-linked but is not externally signed or independently anchored.".to_owned(),
    ];
    if coverage.unsupported_files > 0 {
        limitations.push(format!(
            "{} files ({} bytes) were retained as immutable evidence but not structurally interpreted.",
            coverage.unsupported_files, coverage.unsupported_bytes
        ));
    }
    AuditCoverageView {
        schema_version: "qbm.audit-coverage/v1",
        audit_scope: "generic_source",
        completion_level: "source_only",
        completion_message: "Generic source audit complete.",
        checks,
        limitations,
        next_required_stage: Some("biomedical_profile"),
    }
}

fn report_check(
    report: Option<&AuditReport>,
    check_id: &'static str,
    code: &str,
    explanation: &str,
) -> CheckResultView {
    let matching = report
        .into_iter()
        .flat_map(|report| report.findings.iter())
        .filter(|finding| finding.code == code)
        .collect::<Vec<_>>();
    let status = matching
        .iter()
        .map(|finding| finding.severity)
        .max()
        .map_or("pass", severity_status);
    CheckResultView {
        check_id,
        family: "generic_structure",
        status: if matching.is_empty() && report.is_none() {
            "not_started"
        } else {
            status
        },
        findings: matching.len() as u64,
        explanation: explanation.to_owned(),
        skip_reason: report
            .is_none()
            .then(|| "The generic audit has not run yet.".to_owned()),
    }
}

fn severity_status(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "observations",
        Severity::Warning => "warning",
        Severity::Error | Severity::Critical => "fail",
    }
}

fn inventory_view(value: &qbm_domain::Inventory) -> InventoryView {
    InventoryView {
        id: value.id,
        inventory_hash: value.inventory_hash.clone(),
        source_kind: value.source_kind,
        source_container_hash: value.source_container_hash.clone(),
        included_files: value.included_files,
        excluded_entries: value.excluded_entries,
        blocked_entries: value.blocked_entries,
        total_included_bytes: value.total_included_bytes,
        exceptions: value
            .entries
            .iter()
            .filter(|entry| entry.disposition != InventoryDisposition::Included)
            .take(200)
            .cloned()
            .collect(),
    }
}

fn browser_intake_policy() -> IntakePolicy {
    IntakePolicy {
        max_entries: 200_000,
        max_total_bytes: 5 * 1024 * 1024 * 1024,
        max_single_file_bytes: 256 * 1024 * 1024,
        max_depth: 64,
        max_compression_ratio: 200,
        ..IntakePolicy::default()
    }
}

fn normalized_display_name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 160 || value.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "invalid_project_name",
            "Use a project name between 1 and 160 characters.",
        ));
    }
    Ok(value.to_owned())
}

fn next_project_id(app: &PlatformApp, display_name: &str) -> Result<String, ApiError> {
    let mut slug = String::new();
    let mut previous_separator = false;
    for character in display_name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator && !slug.is_empty() {
            slug.push('-');
            previous_separator = true;
        }
        if slug.len() >= 52 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("project");
    }
    let existing = app
        .projects()?
        .into_iter()
        .map(|project| project.id.to_string())
        .collect::<BTreeSet<_>>();
    if !existing.contains(&slug) {
        return Ok(slug);
    }
    for suffix in 2..=999_999_u32 {
        let candidate = format!("{slug}-{suffix}");
        if !existing.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(ApiError::conflict(
        "project_id_exhausted",
        "Could not allocate a unique project identifier.",
    ))
}

fn missing_output(name: &str) -> ApiError {
    ApiError::internal(format!("The completed run is missing its {name} output."))
}
