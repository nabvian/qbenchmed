//! End-to-end approval-gating coverage for the governed adapter pipeline.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use qbm_app::{AppError, PlatformApp};
use qbm_domain::{
    AdapterDetectionReport, AdapterPlan, AdapterRecord, Decision, GraphPolicy, IntakePolicy,
    Inventory, Project, ProjectionCatalog, RawProjectGraph, Run, RunEvent, RunId, RunMode,
    RunState, SemanticAuditReport, SemanticGraph, Sha256Digest, SourceSnapshot,
};
use qbm_store::StoreError;
use tempfile::TempDir;

const GENERIC_ADAPTER_PACKAGE: &[u8] =
    include_bytes!("../../../examples/adapters/generic-manifest-v1.json");

fn generic_adapter_path(directory: &TempDir) -> PathBuf {
    let path = directory.path().join("generic-manifest-v1.json");
    fs::write(&path, GENERIC_ADAPTER_PACKAGE).unwrap();
    path
}

fn approve(app: &PlatformApp, run_id: RunId, stage_id: &str, output: &Sha256Digest) {
    app.decide_stage(
        run_id,
        stage_id,
        output.as_str(),
        Decision::Approve,
        "integration-reviewer",
        Some(format!("approved {stage_id}")),
    )
    .unwrap();
}

fn assert_approval_gate<T: std::fmt::Debug>(
    result: Result<T, AppError>,
    expected_stage: &str,
    expected_hash: &Sha256Digest,
) {
    match result {
        Err(AppError::Store(StoreError::RequiredApprovalMissing {
            stage_id,
            expected_hash: actual_hash,
            ..
        })) => {
            assert_eq!(stage_id.to_string(), expected_stage);
            assert_eq!(&actual_hash, expected_hash);
        }
        other => panic!("expected approval gate for {expected_stage}, got {other:?}"),
    }
}

struct PipelineRecords<'a> {
    project: &'a Project,
    run: &'a Run,
    inventory: &'a Inventory,
    snapshot: &'a SourceSnapshot,
    graph: &'a RawProjectGraph,
    adapter: &'a AdapterRecord,
    detection: &'a AdapterDetectionReport,
    plan: &'a AdapterPlan,
    semantic_graph: &'a SemanticGraph,
    semantic_report: &'a SemanticAuditReport,
    catalog: &'a ProjectionCatalog,
}

fn approval_history_and_events(app: &PlatformApp, run_id: RunId) -> Vec<RunEvent> {
    let approved_stages = app
        .approvals(run_id)
        .unwrap()
        .into_iter()
        .map(|approval| approval.stage_id.to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        approved_stages,
        BTreeSet::from([
            "adapter-detection".to_owned(),
            "adapter-plan".to_owned(),
            "inventory".to_owned(),
            "raw-graph".to_owned(),
            "semantic-audit".to_owned(),
            "semantic-graph".to_owned(),
            "source-snapshot".to_owned(),
        ])
    );

    let events = app.events(run_id).unwrap();
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.sequence, index as u64 + 1);
        let expected_previous = index.checked_sub(1).map(|prior| &events[prior].event_hash);
        assert_eq!(event.previous_hash.as_ref(), expected_previous);
    }
    events
}

fn assert_pipeline_reopens(
    data_directory: &Path,
    records: &PipelineRecords<'_>,
    expected_events: &[RunEvent],
) {
    let reopened = PlatformApp::open(data_directory).unwrap();
    let reopened_project = reopened.project(records.project.id.as_str()).unwrap();
    assert_eq!(reopened_project.id, records.project.id);
    assert_eq!(reopened_project.display_name, records.project.display_name);
    assert_eq!(reopened_project.source_path, records.project.source_path);
    let reopened_run = reopened.run(records.run.id).unwrap();
    assert_eq!(reopened_run.id, records.run.id);
    assert_eq!(reopened_run.project_id, records.run.project_id);
    assert_eq!(reopened_run.mode, records.run.mode);
    assert_eq!(reopened_run.state, RunState::WaitingApproval);
    assert_eq!(
        &reopened.inventory(records.inventory.id).unwrap(),
        records.inventory
    );
    assert_eq!(
        &reopened.snapshot(records.snapshot.id.as_str()).unwrap(),
        records.snapshot
    );
    assert_eq!(
        &reopened.graph(records.graph.id.as_str()).unwrap(),
        records.graph
    );
    assert_eq!(
        &reopened
            .adapter(records.adapter.package_hash.as_str())
            .unwrap(),
        records.adapter
    );
    assert_eq!(
        &reopened
            .adapter_detection(records.detection.id.as_str())
            .unwrap(),
        records.detection
    );
    assert_eq!(
        &reopened.adapter_plan(records.plan.id.as_str()).unwrap(),
        records.plan
    );
    assert_eq!(
        &reopened
            .semantic_graph(records.semantic_graph.id.as_str())
            .unwrap(),
        records.semantic_graph
    );
    assert_eq!(
        &reopened
            .semantic_report(records.semantic_report.id.as_str())
            .unwrap(),
        records.semantic_report
    );
    assert_eq!(
        &reopened
            .projection_catalog(records.catalog.id.as_str())
            .unwrap(),
        records.catalog
    );
    assert_eq!(reopened.events(records.run.id).unwrap(), expected_events);
    reopened.verify_events(records.run.id).unwrap();
}

#[test]
fn governed_adapter_pipeline_enforces_every_material_approval_and_reopens() {
    let data = TempDir::new().unwrap();
    let source = TempDir::new().unwrap();
    let package_directory = TempDir::new().unwrap();
    let package_path = generic_adapter_path(&package_directory);
    fs::write(
        source.path().join("rules.yaml"),
        b"definitions:\n  loop-rule:\n    id: loop-rule\n    ref: loop-rule\n",
    )
    .unwrap();

    let app = PlatformApp::open(data.path()).unwrap();
    let project = app
        .register_project("adapter-e2e", "Adapter end to end", source.path())
        .unwrap();
    let run = app
        .start_run(project.id.as_str(), RunMode::Governed)
        .unwrap();

    let inventory = app.scan_intake(run.id, &IntakePolicy::default()).unwrap();
    let blocked = app.create_snapshot(run.id, inventory.id);
    assert_approval_gate(blocked, "inventory", &inventory.inventory_hash);
    approve(&app, run.id, "inventory", &inventory.inventory_hash);

    let snapshot = app.create_snapshot(run.id, inventory.id).unwrap();
    let blocked = app.build_graph(run.id, snapshot.id.as_str(), &GraphPolicy::default());
    assert_approval_gate(blocked, "source-snapshot", &snapshot.id);
    approve(&app, run.id, "source-snapshot", &snapshot.id);

    let graph = app
        .build_graph(run.id, snapshot.id.as_str(), &GraphPolicy::default())
        .unwrap();
    let blocked = app.detect_adapters(run.id, graph.id.as_str());
    assert_approval_gate(blocked, "raw-graph", &graph.id);
    approve(&app, run.id, "raw-graph", &graph.id);

    let adapter = app.install_adapter(&package_path).unwrap();
    let detection = app.detect_adapters(run.id, graph.id.as_str()).unwrap();
    assert_eq!(detection.candidates.len(), 1);
    assert!(detection.candidates[0].eligible);
    assert_eq!(detection.candidates[0].package_hash, adapter.package_hash);

    let selected_packages = vec![adapter.package_hash.to_string()];
    let blocked = app.create_adapter_plan(run.id, detection.id.as_str(), &selected_packages);
    assert_approval_gate(blocked, "adapter-detection", &detection.id);
    approve(&app, run.id, "adapter-detection", &detection.id);

    let plan = app
        .create_adapter_plan(run.id, detection.id.as_str(), &selected_packages)
        .unwrap();
    let blocked = app.build_semantic_graph(run.id, plan.id.as_str());
    assert_approval_gate(blocked, "adapter-plan", &plan.id);
    approve(&app, run.id, "adapter-plan", &plan.id);

    let semantic_graph = app.build_semantic_graph(run.id, plan.id.as_str()).unwrap();
    assert_eq!(semantic_graph.nodes.len(), 1);
    assert_eq!(semantic_graph.edges.len(), 1);
    let blocked = app.run_semantic_audit(run.id, semantic_graph.id.as_str());
    assert_approval_gate(blocked, "semantic-graph", &semantic_graph.id);
    approve(&app, run.id, "semantic-graph", &semantic_graph.id);

    let semantic_report = app
        .run_semantic_audit(run.id, semantic_graph.id.as_str())
        .unwrap();
    assert_eq!(semantic_report.summary.warnings, 1);
    assert_approval_gate(
        app.build_projection_catalog(
            run.id,
            semantic_graph.id.as_str(),
            semantic_report.id.as_str(),
        ),
        "semantic-audit",
        &semantic_report.id,
    );
    approve(&app, run.id, "semantic-audit", &semantic_report.id);

    let catalog = app
        .build_projection_catalog(
            run.id,
            semantic_graph.id.as_str(),
            semantic_report.id.as_str(),
        )
        .unwrap();
    assert_eq!(catalog.projections.len(), 1);
    assert!(catalog.projections[0].available);

    let events = approval_history_and_events(&app, run.id);
    drop(app);
    assert_pipeline_reopens(
        data.path(),
        &PipelineRecords {
            project: &project,
            run: &run,
            inventory: &inventory,
            snapshot: &snapshot,
            graph: &graph,
            adapter: &adapter,
            detection: &detection,
            plan: &plan,
            semantic_graph: &semantic_graph,
            semantic_report: &semantic_report,
            catalog: &catalog,
        },
        &events,
    );
}

#[test]
fn generic_adapter_conformance_can_pass_without_installing() {
    let data = TempDir::new().unwrap();
    let package_directory = TempDir::new().unwrap();
    let package_path = generic_adapter_path(&package_directory);
    let app = PlatformApp::open(data.path()).unwrap();

    let report = app.check_adapter(&package_path).unwrap();

    assert!(report.passed, "conformance issues: {:?}", report.issues);
    assert!(report.checks > 0);
    assert_eq!(app.adapter_conformance(report.id.as_str()).unwrap(), report);
    assert!(app.adapters().unwrap().is_empty());
}
