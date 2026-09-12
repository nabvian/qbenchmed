//! Public-API conformance, determinism, provenance, and resource-bound tests.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Duration, Utc};
use qbm_adapter::{AdapterEngine, AdapterError, AdapterLimits};
use qbm_canonical::hash_value;
use qbm_domain::{
    AdapterCapability, AdapterPackage, AdapterRecord, EvidenceLocation, GraphEdge, GraphEdgeKind,
    GraphNode, GraphNodeKind, MappingAttribute, MappingValueSource, ProjectId, RawProjectGraph,
    Sha256Digest, SnapshotArtifact, SourceFormat, SourceSnapshot,
};
use qbm_store::{PlatformStore, StoreError};
use serde_json::{Value, json};
use tempfile::TempDir;

const GENERIC_PACKAGE: &[u8] =
    include_bytes!("../../../examples/adapters/generic-manifest-v1.json");

struct InstalledAdapter {
    _temporary_directory: TempDir,
    store: PlatformStore,
    engine: AdapterEngine,
    package: AdapterPackage,
    record: AdapterRecord,
}

fn digest(label: &str) -> Sha256Digest {
    hash_value(&("qbm-adapter-test/v1", label)).expect("test identity must be hashable")
}

fn new_store() -> (TempDir, PlatformStore) {
    let temporary_directory = tempfile::tempdir().expect("temporary directory must be created");
    let store = PlatformStore::open(temporary_directory.path()).expect("store must open");
    (temporary_directory, store)
}

fn install_bytes(bytes: &[u8]) -> InstalledAdapter {
    let (temporary_directory, store) = new_store();
    let engine = AdapterEngine::new(store.clone());
    let package = engine
        .parse_package_json(bytes)
        .expect("adapter package must parse");
    let report = engine
        .conformance_report(&package)
        .expect("conformance must execute");
    assert!(report.passed, "conformance issues: {:?}", report.issues);
    let source = store
        .put_bytes(
            bytes,
            Some("adapter.json".to_owned()),
            Some("application/json".to_owned()),
        )
        .expect("package source must be stored");
    let record = AdapterRecord {
        package_hash: report.package_hash.clone(),
        source_hash: source.sha256,
        conformance_report_id: report.id,
        package: package.clone(),
        installed_at: Utc::now(),
    };
    InstalledAdapter {
        _temporary_directory: temporary_directory,
        store,
        engine,
        package,
        record,
    }
}

fn install_package(package: &AdapterPackage) -> InstalledAdapter {
    let bytes = serde_json::to_vec(package).expect("test package must serialize");
    install_bytes(&bytes)
}

fn generic_adapter() -> InstalledAdapter {
    install_bytes(GENERIC_PACKAGE)
}

fn project_id() -> ProjectId {
    ProjectId::new("adapter-tests").expect("test project ID must be valid")
}

fn snapshot(artifacts: Vec<SnapshotArtifact>) -> SourceSnapshot {
    let project_id = project_id();
    let inventory_hash = digest("inventory");
    let policy_hash = digest("intake-policy");
    let total_bytes = artifacts.iter().map(|artifact| artifact.size_bytes).sum();
    let id = hash_value(&(
        "qbm-adapter-test-snapshot/v1",
        &inventory_hash,
        &policy_hash,
        &project_id,
        &artifacts,
    ))
    .expect("snapshot identity must be hashable");
    SourceSnapshot {
        schema_version: qbm_domain::DOMAIN_SCHEMA_VERSION.to_owned(),
        id,
        inventory_hash,
        policy_hash,
        project_id,
        artifacts,
        total_bytes,
        created_at: Utc::now(),
    }
}

fn definition_node(
    key: &str,
    identifier: &str,
    artifact_sha256: Option<Sha256Digest>,
    relative_path: &str,
    pointer: Option<&str>,
) -> GraphNode {
    GraphNode {
        id: digest(&format!("node-{key}")),
        kind: GraphNodeKind::Object,
        label: identifier.to_owned(),
        format: SourceFormat::Json,
        identifier: Some(identifier.to_owned()),
        is_definition: true,
        properties: BTreeMap::new(),
        evidence: EvidenceLocation {
            artifact_sha256,
            relative_path: Some(relative_path.to_owned()),
            pointer: pointer.map(str::to_owned),
            line: None,
            column: None,
        },
    }
}

fn unrelated_node(key: &str) -> GraphNode {
    GraphNode {
        id: digest(&format!("node-{key}")),
        kind: GraphNodeKind::File,
        label: "README.txt".to_owned(),
        format: SourceFormat::Unknown,
        identifier: None,
        is_definition: false,
        properties: BTreeMap::new(),
        evidence: EvidenceLocation {
            artifact_sha256: None,
            relative_path: Some("README.txt".to_owned()),
            pointer: None,
            line: None,
            column: None,
        },
    }
}

fn reference_edge(key: &str, node: &GraphNode) -> GraphEdge {
    GraphEdge {
        id: digest(&format!("edge-{key}")),
        from: node.id.clone(),
        to: Some(node.id.clone()),
        kind: GraphEdgeKind::References,
        reference: node.identifier.clone(),
        evidence: node.evidence.clone(),
    }
}

fn graph(
    source_snapshot: &SourceSnapshot,
    mut nodes: Vec<GraphNode>,
    mut edges: Vec<GraphEdge>,
) -> RawProjectGraph {
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    let policy_hash = digest("graph-policy");
    let id = hash_value(&(
        "qbm-adapter-test-raw-graph/v1",
        &source_snapshot.id,
        &source_snapshot.project_id,
        &policy_hash,
        &nodes,
        &edges,
    ))
    .expect("raw graph identity must be hashable");
    RawProjectGraph {
        schema_version: qbm_domain::DOMAIN_SCHEMA_VERSION.to_owned(),
        id,
        snapshot_id: source_snapshot.id.clone(),
        project_id: source_snapshot.project_id.clone(),
        policy_hash,
        nodes,
        edges,
        diagnostics: Vec::new(),
        created_at: Utc::now(),
    }
}

fn assert_resource_limit(error: AdapterError, expected_kind: &str) {
    match error {
        AdapterError::ResourceLimit { kind, .. } => assert_eq!(kind, expected_kind),
        other => panic!("expected {expected_kind:?} resource limit, got {other:?}"),
    }
}

#[test]
fn generic_package_passes_positive_and_negative_fixture_conformance() {
    let installed = generic_adapter();
    let package = &installed.package;
    assert!(
        package
            .fixtures
            .iter()
            .any(|fixture| fixture.expect.detection_eligible)
    );
    assert!(
        package
            .fixtures
            .iter()
            .any(|fixture| !fixture.expect.detection_eligible)
    );

    let first = installed
        .engine
        .conformance_report(package)
        .expect("conformance must execute");
    let second = installed
        .engine
        .conformance_report(package)
        .expect("conformance must repeat");
    assert!(first.passed);
    assert!(first.issues.is_empty());
    assert!(first.checks > 0);
    assert_eq!(first.id, second.id, "timestamps must not affect identity");

    let mut wrong_expectation = package.clone();
    let negative = wrong_expectation
        .fixtures
        .iter_mut()
        .find(|fixture| !fixture.expect.detection_eligible)
        .expect("generic package must contain a negative fixture");
    negative.expect.minimum_confidence_bps = 1;
    let failed = installed
        .engine
        .conformance_report(&wrong_expectation)
        .expect("a fixture mismatch must produce a report");
    assert!(!failed.passed);
    assert!(failed.issues.iter().any(|issue| {
        issue.code == "fixture_detection_confidence"
            && issue.fixture_id.as_deref() == Some("unrelated-text-file")
    }));
}

#[test]
fn strict_json_rejects_duplicate_keys_unknown_fields_and_trailing_content() {
    let installed = generic_adapter();
    let source = std::str::from_utf8(GENERIC_PACKAGE).expect("fixture must be UTF-8");
    let duplicate = source.replacen(
        "\"schema_version\": \"qbm.adapter-package/v1\",",
        "\"schema_version\": \"qbm.adapter-package/v1\",\n  \"schema_version\": \"qbm.adapter-package/v1\",",
        1,
    );
    let error = installed
        .engine
        .parse_package_json(duplicate.as_bytes())
        .expect_err("duplicate keys must be rejected");
    assert!(error.to_string().contains("duplicate object key"));

    let mut value: Value =
        serde_json::from_slice(GENERIC_PACKAGE).expect("fixture JSON must parse");
    value
        .as_object_mut()
        .expect("package must be an object")
        .insert("execute".to_owned(), json!("malicious-command"));
    let unknown = serde_json::to_vec(&value).expect("modified fixture must serialize");
    let error = installed
        .engine
        .parse_package_json(&unknown)
        .expect_err("unknown fields must be rejected");
    assert!(error.to_string().contains("unknown field"));

    let mut trailing = GENERIC_PACKAGE.to_vec();
    trailing.extend_from_slice(b" true");
    assert!(installed.engine.parse_package_json(&trailing).is_err());
}

#[test]
fn conformance_rejects_semantics_outside_the_adapter_namespace() {
    let installed = generic_adapter();
    let mut package = installed.package.clone();
    package.mappings.nodes[0].semantic_type = "foreign::definition".to_owned();

    let report = installed
        .engine
        .conformance_report(&package)
        .expect("invalid packages still receive a report");
    assert!(!report.passed);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "semantic_type_namespace")
    );
}

#[test]
fn detection_is_bounded_evidence_backed_and_enforces_eligibility() {
    let installed = generic_adapter();
    let source_snapshot = snapshot(Vec::new());
    let first = definition_node("first", "first", None, "rules.json", Some("/rules/first"));
    let second = definition_node(
        "second",
        "second",
        None,
        "rules.json",
        Some("/rules/second"),
    );
    let matching_ids = [first.id.clone(), second.id.clone()];
    let matching_graph = graph(&source_snapshot, vec![second, first], Vec::new());
    let limits = AdapterLimits {
        max_detection_evidence_samples: 1,
        ..AdapterLimits::default()
    };
    let bounded_engine = AdapterEngine::with_limits(installed.store.clone(), limits);
    let report = bounded_engine
        .detect(&matching_graph, std::slice::from_ref(&installed.record))
        .expect("detection must execute");
    let candidate = &report.candidates[0];
    assert!(candidate.eligible);
    assert_eq!(candidate.confidence_bps, 10_000);
    assert_eq!(candidate.rules[0].match_count, 2);
    assert_eq!(candidate.rules[0].matching_node_samples.len(), 1);
    assert_eq!(
        candidate.rules[0].matching_node_samples[0],
        matching_ids.into_iter().min().expect("two IDs exist")
    );

    let nonmatching_graph = graph(&source_snapshot, vec![unrelated_node("readme")], Vec::new());
    let nonmatching = bounded_engine
        .detect(&nonmatching_graph, std::slice::from_ref(&installed.record))
        .expect("negative detection must execute");
    assert!(!nonmatching.candidates[0].eligible);
    assert_eq!(nonmatching.candidates[0].confidence_bps, 0);
    let error = bounded_engine
        .create_plan(
            &nonmatching,
            std::slice::from_ref(&installed.record.package_hash),
            std::slice::from_ref(&installed.record),
        )
        .expect_err("an ineligible adapter cannot be selected");
    assert!(matches!(error, AdapterError::InvalidRelationship(_)));
}

#[test]
fn records_require_exact_source_provenance_and_plans_reject_duplicate_selection() {
    let installed = generic_adapter();
    let source_snapshot = snapshot(Vec::new());
    let node = definition_node("definition", "definition", None, "rules.json", None);
    let raw_graph = graph(&source_snapshot, vec![node], Vec::new());
    let detection = installed
        .engine
        .detect(&raw_graph, std::slice::from_ref(&installed.record))
        .expect("detection must execute");
    let duplicate = [
        installed.record.package_hash.clone(),
        installed.record.package_hash.clone(),
    ];
    let error = installed
        .engine
        .create_plan(
            &detection,
            &duplicate,
            std::slice::from_ref(&installed.record),
        )
        .expect_err("duplicate package selections must be rejected");
    assert!(matches!(error, AdapterError::InvalidRelationship(_)));

    let mut different_package = installed.package.clone();
    different_package.manifest.description.push_str(" Changed.");
    let different_source =
        serde_json::to_vec(&different_package).expect("modified package source must serialize");
    let artifact = installed
        .store
        .put_bytes(
            &different_source,
            Some("different.json".to_owned()),
            Some("application/json".to_owned()),
        )
        .expect("modified package source must be stored");
    let mut forged = installed.record.clone();
    forged.source_hash = artifact.sha256;
    let error = installed
        .engine
        .detect(&raw_graph, &[forged])
        .expect_err("source bytes must decode to the exact registered package");
    match error {
        AdapterError::InvalidRelationship(message) => {
            assert!(message.contains("does not decode to registered package"));
        }
        other => panic!("expected source-provenance rejection, got {other:?}"),
    }
}

#[test]
fn semantic_pipeline_is_deterministic_and_timestamps_are_non_identifying() {
    let installed = generic_adapter();
    let source_snapshot = snapshot(Vec::new());
    let node = definition_node("loop", "loop", None, "rules.json", Some("/rules/loop"));
    let edge = reference_edge("loop", &node);
    let raw_graph = graph(&source_snapshot, vec![node], vec![edge]);
    let records = std::slice::from_ref(&installed.record);

    let detection = installed
        .engine
        .detect(&raw_graph, records)
        .expect("detection must execute");
    let repeated_detection = installed
        .engine
        .detect(&raw_graph, records)
        .expect("detection must repeat");
    assert_eq!(detection.id, repeated_detection.id);
    let mut later_detection = detection.clone();
    later_detection.created_at += Duration::days(1);

    let selected = std::slice::from_ref(&installed.record.package_hash);
    let plan = installed
        .engine
        .create_plan(&detection, selected, records)
        .expect("plan must be created");
    let plan_from_later_detection = installed
        .engine
        .create_plan(&later_detection, selected, records)
        .expect("detection timestamps must not invalidate a plan");
    assert_eq!(plan.id, plan_from_later_detection.id);
    let duplicate_plan = installed
        .engine
        .create_plan(&detection, selected, records)
        .expect("plan creation must be deterministic");
    assert_eq!(plan.id, duplicate_plan.id);
    let mut later_plan = plan.clone();
    later_plan.created_at += Duration::days(1);

    let semantic = installed
        .engine
        .project_with_store(&raw_graph, &source_snapshot, &plan, records)
        .expect("semantic graph must be projected");
    let repeated_semantic = installed
        .engine
        .project_with_store(&raw_graph, &source_snapshot, &later_plan, records)
        .expect("plan timestamps must not affect projection");
    assert_eq!(semantic.id, repeated_semantic.id);
    assert_eq!(semantic.nodes.len(), 1);
    assert_eq!(semantic.edges.len(), 1);
    assert_eq!(semantic.nodes[0].external_id, "loop");
    assert_eq!(
        semantic.nodes[0].semantic_type,
        "generic-manifest::definition"
    );
    let mut later_semantic = semantic.clone();
    later_semantic.created_at += Duration::days(1);

    let audit = installed
        .engine
        .audit(&semantic, &plan, records)
        .expect("semantic policies must execute");
    let repeated_audit = installed
        .engine
        .audit(&later_semantic, &plan, records)
        .expect("semantic graph timestamps must not affect audit");
    assert_eq!(audit.id, repeated_audit.id);
    assert_eq!(audit.summary.total, 1);
    assert_eq!(audit.summary.warnings, 1);
    assert_eq!(
        audit.findings[0].code,
        "generic-manifest::forbid-self-reference"
    );
    let mut later_audit = audit.clone();
    later_audit.created_at += Duration::days(1);

    let catalog = installed
        .engine
        .resolve_catalog(&semantic, &audit, &plan, records)
        .expect("projection catalog must resolve");
    let repeated_catalog = installed
        .engine
        .resolve_catalog(&semantic, &later_audit, &plan, records)
        .expect("audit timestamps must not affect projection identity");
    assert_eq!(catalog.id, repeated_catalog.id);
    assert_eq!(catalog.projections.len(), 1);
    assert!(catalog.projections[0].available);
    assert_eq!(
        catalog.projections[0].nodes,
        vec![semantic.nodes[0].id.clone()]
    );
    assert_eq!(
        catalog.projections[0].edges,
        vec![semantic.edges[0].id.clone()]
    );
}

fn evidence_package() -> AdapterPackage {
    let installed = generic_adapter();
    let mut package = installed.package;
    package
        .manifest
        .capabilities
        .insert(AdapterCapability::SnapshotArtifactTextRead);
    package.mappings.nodes[0].attributes.push(MappingAttribute {
        target: "payload".to_owned(),
        value: MappingValueSource::EvidenceValue,
        required: true,
    });
    package.fixtures[0].nodes[0].evidence_value = Some(json!({"id": "loop-rule"}));
    package
}

#[test]
fn evidence_values_are_confined_to_the_exact_snapshot_and_bounded_reads() {
    let installed = install_package(&evidence_package());
    let document = br#"{"rules":{"alpha":{"id":"alpha","score":88}}}"#;
    let artifact = installed
        .store
        .put_bytes(
            document,
            Some("rules.json".to_owned()),
            Some("application/json".to_owned()),
        )
        .expect("evidence document must be stored");
    let source_snapshot = snapshot(vec![SnapshotArtifact {
        relative_path: "rules.json".to_owned(),
        sha256: artifact.sha256.clone(),
        size_bytes: artifact.size_bytes,
    }]);
    let node = definition_node(
        "alpha",
        "alpha",
        Some(artifact.sha256.clone()),
        "rules.json",
        Some("/rules/alpha"),
    );
    let raw_graph = graph(&source_snapshot, vec![node], Vec::new());
    let records = std::slice::from_ref(&installed.record);
    let detection = installed
        .engine
        .detect(&raw_graph, records)
        .expect("detection must execute");
    let plan = installed
        .engine
        .create_plan(
            &detection,
            std::slice::from_ref(&installed.record.package_hash),
            records,
        )
        .expect("plan must be created");
    let semantic = installed
        .engine
        .project_with_store(&raw_graph, &source_snapshot, &plan, records)
        .expect("snapshot-bound evidence must resolve");
    assert_eq!(
        semantic.nodes[0].attributes.get("payload"),
        Some(&json!({"id": "alpha", "score": 88}))
    );

    let outside_node = definition_node(
        "outside",
        "outside",
        Some(artifact.sha256.clone()),
        "not-in-snapshot.json",
        Some("/rules/alpha"),
    );
    let outside_graph = graph(&source_snapshot, vec![outside_node], Vec::new());
    let outside_detection = installed
        .engine
        .detect(&outside_graph, records)
        .expect("detection must not read evidence");
    let outside_plan = installed
        .engine
        .create_plan(
            &outside_detection,
            std::slice::from_ref(&installed.record.package_hash),
            records,
        )
        .expect("eligible adapter may be planned before evidence resolution");
    let error = installed
        .engine
        .project_with_store(&outside_graph, &source_snapshot, &outside_plan, records)
        .expect_err("an artifact/path pair outside the snapshot must be rejected");
    assert!(matches!(error, AdapterError::Evidence(_)));

    let limits = AdapterLimits {
        max_evidence_bytes: artifact.size_bytes.saturating_sub(1),
        ..AdapterLimits::default()
    };
    let bounded_engine = AdapterEngine::with_limits(installed.store.clone(), limits);
    let error = bounded_engine
        .project_with_store(&raw_graph, &source_snapshot, &plan, records)
        .expect_err("oversized evidence must be rejected before reading");
    assert_resource_limit(error, "evidence artifact bytes");
}

#[test]
fn package_fuel_output_registry_and_source_read_limits_are_enforced() {
    let installed = generic_adapter();

    let package_limit = AdapterLimits {
        max_package_bytes: GENERIC_PACKAGE.len().saturating_sub(1),
        ..AdapterLimits::default()
    };
    let package_limited = AdapterEngine::with_limits(installed.store.clone(), package_limit);
    let error = package_limited
        .parse_package_json(GENERIC_PACKAGE)
        .expect_err("oversized package JSON must be rejected");
    assert_resource_limit(error, "package bytes");

    let fuel_limited = AdapterEngine::with_limits(
        installed.store.clone(),
        AdapterLimits {
            max_rule_evaluations: 0,
            ..AdapterLimits::default()
        },
    );
    let error = fuel_limited
        .conformance_report(&installed.package)
        .expect_err("fixture interpretation must consume bounded fuel");
    assert_resource_limit(error, "rule evaluations");

    let output_limited = AdapterEngine::with_limits(
        installed.store.clone(),
        AdapterLimits {
            max_total_semantic_output_bytes: 1,
            ..AdapterLimits::default()
        },
    );
    let error = output_limited
        .conformance_report(&installed.package)
        .expect_err("fixture semantic output must be bounded");
    assert_resource_limit(error, "cumulative semantic output bytes");

    let declaration_limited = AdapterEngine::with_limits(
        installed.store.clone(),
        AdapterLimits {
            max_detection_rules: 0,
            ..AdapterLimits::default()
        },
    );
    let report = declaration_limited
        .conformance_report(&installed.package)
        .expect("static limit violations must produce a conformance report");
    assert!(!report.passed);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "detection_rule_limit")
    );

    let source_snapshot = snapshot(Vec::new());
    let raw_graph = graph(&source_snapshot, vec![unrelated_node("readme")], Vec::new());
    let error = package_limited
        .detect(&raw_graph, std::slice::from_ref(&installed.record))
        .expect_err("registry package source reads must use the package byte bound");
    assert!(matches!(
        error,
        AdapterError::Store(StoreError::ArtifactReadLimit { .. })
    ));

    let registry_limited = AdapterEngine::with_limits(
        installed.store.clone(),
        AdapterLimits {
            max_registry_packages: 0,
            ..AdapterLimits::default()
        },
    );
    let error = registry_limited
        .detect(&raw_graph, std::slice::from_ref(&installed.record))
        .expect_err("registry cardinality must be bounded");
    assert_resource_limit(error, "registry packages");

    let detection_output_limited = AdapterEngine::with_limits(
        installed.store.clone(),
        AdapterLimits {
            max_total_detection_output_bytes: 1,
            ..AdapterLimits::default()
        },
    );
    let error = detection_output_limited
        .detect(&raw_graph, std::slice::from_ref(&installed.record))
        .expect_err("aggregate detection output must be byte bounded");
    assert_resource_limit(error, "cumulative detection output bytes");
}

#[test]
fn package_hash_is_independent_of_json_object_key_order() {
    let installed = generic_adapter();
    let canonical_hash = installed
        .engine
        .package_hash(&installed.package)
        .expect("package must be hashable");
    let compact = serde_json::to_vec(&installed.package).expect("package must serialize");
    let reparsed = installed
        .engine
        .parse_package_json(&compact)
        .expect("reordered package JSON must parse");
    assert_eq!(
        canonical_hash,
        installed
            .engine
            .package_hash(&reparsed)
            .expect("reparsed package must be hashable")
    );
    assert_eq!(
        installed.record.package_hash, canonical_hash,
        "registry lock must use normalized package content, not source bytes"
    );
    assert_ne!(
        installed.record.source_hash, canonical_hash,
        "the example's pretty JSON source hash should remain separate provenance"
    );
}

#[test]
fn adapter_capabilities_are_an_allowlist_without_execution_primitive() {
    let installed = generic_adapter();
    let serialized = serde_json::to_string(&installed.package.manifest.capabilities)
        .expect("capabilities must serialize");
    for forbidden in ["process", "network", "shell", "native", "dynamic", "script"] {
        assert!(!serialized.contains(forbidden));
    }
    assert_eq!(
        installed.package.manifest.capabilities,
        BTreeSet::from([
            AdapterCapability::RawGraphRead,
            AdapterCapability::EmitSemanticNodes,
            AdapterCapability::EmitSemanticEdges,
            AdapterCapability::EmitFindings,
            AdapterCapability::EmitProjections,
        ])
    );
}
