use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use chrono::Utc;
use qbm_canonical::hash_value;
use qbm_domain::{
    AuditFinding, AuditReport, AuditSummary, DOMAIN_SCHEMA_VERSION, EvidenceLocation, GraphEdge,
    GraphEdgeKind, GraphNode, GraphNodeKind, ProjectId, RawProjectGraph, Severity, Sha256Digest,
    SourceFormat,
};
use serde::Serialize;

use crate::AuditError;

#[derive(Debug, Serialize)]
struct FindingIdentity<'a> {
    schema_version: &'static str,
    code: &'a str,
    severity: Severity,
    title: &'a str,
    message: &'a str,
    remediation: &'a str,
    affected_nodes: &'a [Sha256Digest],
    evidence: &'a [EvidenceLocation],
}

#[derive(Debug, Serialize)]
struct ReportIdentity<'a> {
    schema_version: &'static str,
    graph_id: &'a Sha256Digest,
    project_id: &'a ProjectId,
    findings: &'a [AuditFinding],
    summary: &'a AuditSummary,
}

pub(super) fn audit(graph: &RawProjectGraph) -> Result<AuditReport, AuditError> {
    let mut findings = Vec::new();
    findings.extend(diagnostic_findings(graph)?);
    findings.extend(duplicate_identifier_findings(graph)?);
    findings.extend(duplicate_content_findings(graph)?);
    findings.extend(unresolved_reference_findings(graph)?);
    findings.extend(reference_cycle_findings(graph)?);
    findings.extend(unreachable_definition_findings(graph)?);
    if let Some(finding) = provenance_finding(graph)? {
        findings.push(finding);
    }
    findings.sort_by(|left, right| {
        Reverse(left.severity)
            .cmp(&Reverse(right.severity))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.id.cmp(&right.id))
    });
    let summary = summarize(&findings);
    let id = hash_value(&ReportIdentity {
        schema_version: "qbm.audit-report-identity/v1",
        graph_id: &graph.id,
        project_id: &graph.project_id,
        findings: &findings,
        summary: &summary,
    })?;
    Ok(AuditReport {
        schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
        id,
        graph_id: graph.id.clone(),
        project_id: graph.project_id.clone(),
        findings,
        summary,
        created_at: Utc::now(),
    })
}

fn diagnostic_findings(graph: &RawProjectGraph) -> Result<Vec<AuditFinding>, AuditError> {
    graph
        .diagnostics
        .iter()
        .map(|diagnostic| {
            finding(
                format!("scanner.{}", diagnostic.code),
                diagnostic.severity,
                "Source scanner diagnostic".to_owned(),
                diagnostic.message.clone(),
                "Correct the source syntax or adjust the explicit graph resource policy."
                    .to_owned(),
                Vec::new(),
                vec![diagnostic.evidence.clone()],
            )
        })
        .collect()
}

fn duplicate_identifier_findings(graph: &RawProjectGraph) -> Result<Vec<AuditFinding>, AuditError> {
    let mut groups: BTreeMap<(String, String), Vec<&GraphNode>> = BTreeMap::new();
    for node in &graph.nodes {
        let Some(identifier) = &node.identifier else {
            continue;
        };
        if node.kind == GraphNodeKind::Project || !node.is_definition {
            continue;
        }
        let path = node
            .evidence
            .relative_path
            .as_deref()
            .unwrap_or("<unknown>");
        let scope = if matches!(node.format, SourceFormat::Rust | SourceFormat::Python) {
            path.to_owned()
        } else {
            let container = node
                .evidence
                .pointer
                .as_deref()
                .and_then(|pointer| pointer.rsplit_once('/').map(|(parent, _)| parent))
                .unwrap_or("");
            format!("{path}#{container}")
        };
        groups
            .entry((scope, identifier.clone()))
            .or_default()
            .push(node);
    }
    let mut findings = Vec::new();
    for ((scope, identifier), nodes) in groups {
        if nodes.len() < 2 {
            continue;
        }
        findings.push(finding(
            "generic.duplicate_identifier".to_owned(),
            Severity::Error,
            "Duplicate identifier".to_owned(),
            format!(
                "Identifier {identifier:?} is declared {} times in scope {scope:?}.",
                nodes.len()
            ),
            "Assign one stable unique identifier inside this definition container, or use a semantic adapter that declares a narrower namespace."
                .to_owned(),
            nodes.iter().map(|node| node.id.clone()).collect(),
            nodes.iter().map(|node| node.evidence.clone()).collect(),
        )?);
    }
    Ok(findings)
}

fn duplicate_content_findings(graph: &RawProjectGraph) -> Result<Vec<AuditFinding>, AuditError> {
    let mut groups: BTreeMap<Sha256Digest, Vec<&GraphNode>> = BTreeMap::new();
    for node in graph
        .nodes
        .iter()
        .filter(|node| node.kind == GraphNodeKind::File)
    {
        if let Some(hash) = &node.evidence.artifact_sha256 {
            groups.entry(hash.clone()).or_default().push(node);
        }
    }
    let mut findings = Vec::new();
    for (hash, nodes) in groups {
        if nodes.len() < 2 {
            continue;
        }
        findings.push(finding(
            "generic.duplicate_file_content".to_owned(),
            Severity::Info,
            "Duplicate file content".to_owned(),
            format!(
                "{} source paths contain the same SHA-256 content {hash}.",
                nodes.len()
            ),
            "Confirm whether the copies are intentional; consolidate them when one authoritative source is sufficient."
                .to_owned(),
            nodes.iter().map(|node| node.id.clone()).collect(),
            nodes.iter().map(|node| node.evidence.clone()).collect(),
        )?);
    }
    Ok(findings)
}

fn unresolved_reference_findings(graph: &RawProjectGraph) -> Result<Vec<AuditFinding>, AuditError> {
    graph
        .edges
        .iter()
        .filter(|edge| {
            !matches!(edge.kind, GraphEdgeKind::Contains | GraphEdgeKind::Calls)
                && edge.to.is_none()
        })
        .map(|edge| {
            let (severity, title, remediation) = match edge.kind {
                GraphEdgeKind::References => (
                    Severity::Error,
                    "Unresolved explicit reference",
                    "Define the referenced identifier/pointer or correct the reference value.",
                ),
                GraphEdgeKind::DeclaresModule => (
                    Severity::Error,
                    "Unresolved Rust module",
                    "Add the declared module file or correct the module declaration.",
                ),
                GraphEdgeKind::Imports => (
                    Severity::Info,
                    "External or unresolved source import",
                    "Confirm this import resolves through the language dependency graph; a semantic adapter may classify it further.",
                ),
                GraphEdgeKind::Contains | GraphEdgeKind::Calls => {
                    unreachable!("contains and call edges are filtered")
                }
            };
            finding(
                "generic.unresolved_reference".to_owned(),
                severity,
                title.to_owned(),
                format!(
                    "Reference {:?} has no unique target in the raw project graph.",
                    edge.reference.as_deref().unwrap_or("")
                ),
                remediation.to_owned(),
                vec![edge.from.clone()],
                vec![edge.evidence.clone()],
            )
        })
        .collect()
}

fn reference_cycle_findings(graph: &RawProjectGraph) -> Result<Vec<AuditFinding>, AuditError> {
    let semantic_edges = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == GraphEdgeKind::References && edge.to.is_some())
        .collect::<Vec<_>>();
    let components = strongly_connected_components(&semantic_edges);
    let nodes_by_id = graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let mut findings = Vec::new();
    for component in components {
        let self_loop = component.len() == 1
            && semantic_edges
                .iter()
                .any(|edge| edge.from == component[0] && edge.to.as_ref() == Some(&component[0]));
        if component.len() < 2 && !self_loop {
            continue;
        }
        let evidence = component
            .iter()
            .filter_map(|id| nodes_by_id.get(id))
            .map(|node| node.evidence.clone())
            .collect::<Vec<_>>();
        findings.push(finding(
            "generic.reference_cycle".to_owned(),
            Severity::Warning,
            "Reference cycle".to_owned(),
            format!(
                "A strongly connected reference component contains {} node(s).",
                component.len()
            ),
            "Break the cycle or document that the recursive relationship is intentional and supported."
                .to_owned(),
            component,
            evidence,
        )?);
    }
    Ok(findings)
}

fn unreachable_definition_findings(
    graph: &RawProjectGraph,
) -> Result<Vec<AuditFinding>, AuditError> {
    let incoming = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == GraphEdgeKind::References)
        .filter_map(|edge| edge.to.clone())
        .collect::<BTreeSet<_>>();
    graph
        .nodes
        .iter()
        .filter(|node| {
            node.is_definition
                && !matches!(node.format, SourceFormat::Rust | SourceFormat::Python)
                && !incoming.contains(&node.id)
        })
        .map(|node| {
            finding(
                "generic.unreachable_definition".to_owned(),
                Severity::Info,
                "Unreferenced definition".to_owned(),
                format!(
                    "Definition {:?} has no incoming explicit reference.",
                    node.identifier.as_deref().unwrap_or(&node.label)
                ),
                "Reference the definition from an entry flow, remove it if obsolete, or configure a domain adapter that defines additional roots."
                    .to_owned(),
                vec![node.id.clone()],
                vec![node.evidence.clone()],
            )
        })
        .collect()
}

fn provenance_finding(graph: &RawProjectGraph) -> Result<Option<AuditFinding>, AuditError> {
    let mut affected_nodes = graph
        .nodes
        .iter()
        .filter(|node| {
            node.kind != GraphNodeKind::Project
                && (node.evidence.artifact_sha256.is_none()
                    || node.evidence.relative_path.is_none())
        })
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let missing_edges = graph
        .edges
        .iter()
        .filter(|edge| {
            edge.evidence.artifact_sha256.is_none() || edge.evidence.relative_path.is_none()
        })
        .count();
    if affected_nodes.is_empty() && missing_edges == 0 {
        return Ok(None);
    }
    affected_nodes.sort();
    Ok(Some(finding(
        "generic.missing_provenance".to_owned(),
        Severity::Critical,
        "Graph provenance gap".to_owned(),
        format!(
            "{} non-synthetic node(s) and {missing_edges} edge(s) lack immutable source provenance.",
            affected_nodes.len()
        ),
        "Do not consume this graph downstream; correct the scanner so every element points to a frozen artifact and source location."
            .to_owned(),
        affected_nodes,
        Vec::new(),
    )?))
}

fn finding(
    code: String,
    severity: Severity,
    title: String,
    message: String,
    remediation: String,
    mut affected_nodes: Vec<Sha256Digest>,
    mut evidence: Vec<EvidenceLocation>,
) -> Result<AuditFinding, AuditError> {
    affected_nodes.sort();
    affected_nodes.dedup();
    evidence.sort();
    evidence.dedup();
    let id = hash_value(&FindingIdentity {
        schema_version: "qbm.audit-finding-identity/v1",
        code: &code,
        severity,
        title: &title,
        message: &message,
        remediation: &remediation,
        affected_nodes: &affected_nodes,
        evidence: &evidence,
    })?;
    Ok(AuditFinding {
        id,
        code,
        severity,
        title,
        message,
        remediation,
        affected_nodes,
        evidence,
    })
}

fn summarize(findings: &[AuditFinding]) -> AuditSummary {
    AuditSummary {
        total: findings.len() as u64,
        info: findings
            .iter()
            .filter(|finding| finding.severity == Severity::Info)
            .count() as u64,
        warnings: findings
            .iter()
            .filter(|finding| finding.severity == Severity::Warning)
            .count() as u64,
        errors: findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .count() as u64,
        critical: findings
            .iter()
            .filter(|finding| finding.severity == Severity::Critical)
            .count() as u64,
    }
}

fn strongly_connected_components(edges: &[&GraphEdge]) -> Vec<Vec<Sha256Digest>> {
    let mut forward: BTreeMap<Sha256Digest, BTreeSet<Sha256Digest>> = BTreeMap::new();
    let mut reverse: BTreeMap<Sha256Digest, BTreeSet<Sha256Digest>> = BTreeMap::new();
    for edge in edges {
        let Some(to) = &edge.to else {
            continue;
        };
        forward
            .entry(edge.from.clone())
            .or_default()
            .insert(to.clone());
        forward.entry(to.clone()).or_default();
        reverse
            .entry(to.clone())
            .or_default()
            .insert(edge.from.clone());
        reverse.entry(edge.from.clone()).or_default();
    }
    let mut visited = BTreeSet::new();
    let mut order = Vec::new();
    for node in forward.keys() {
        if visited.contains(node) {
            continue;
        }
        finish_order(node, &forward, &mut visited, &mut order);
    }
    visited.clear();
    let mut components = Vec::new();
    for node in order.into_iter().rev() {
        if !visited.insert(node.clone()) {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            component.push(current.clone());
            if let Some(neighbors) = reverse.get(&current) {
                for neighbor in neighbors.iter().rev() {
                    if visited.insert(neighbor.clone()) {
                        stack.push(neighbor.clone());
                    }
                }
            }
        }
        component.sort();
        components.push(component);
    }
    components.sort();
    components
}

fn finish_order(
    start: &Sha256Digest,
    graph: &BTreeMap<Sha256Digest, BTreeSet<Sha256Digest>>,
    visited: &mut BTreeSet<Sha256Digest>,
    order: &mut Vec<Sha256Digest>,
) {
    let mut stack = vec![(start.clone(), false)];
    while let Some((node, expanded)) = stack.pop() {
        if expanded {
            order.push(node);
            continue;
        }
        if !visited.insert(node.clone()) {
            continue;
        }
        stack.push((node.clone(), true));
        if let Some(neighbors) = graph.get(&node) {
            for neighbor in neighbors.iter().rev() {
                if !visited.contains(neighbor) {
                    stack.push((neighbor.clone(), false));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use qbm_domain::{GraphPolicy, SnapshotArtifact};
    use qbm_store::PlatformStore;
    use tempfile::TempDir;

    use crate::builder::build_graph;

    use super::*;

    fn graph_from_yaml(source: &[u8]) -> RawProjectGraph {
        graph_from_files(&[("graph.yaml", source)])
    }

    fn graph_from_files(files: &[(&str, &[u8])]) -> RawProjectGraph {
        let data = TempDir::new().unwrap();
        let store = PlatformStore::open(data.path()).unwrap();
        let artifacts = files
            .iter()
            .map(|(path, source)| {
                let artifact = store
                    .put_bytes(source, Some((*path).to_owned()), None)
                    .unwrap();
                SnapshotArtifact {
                    relative_path: (*path).to_owned(),
                    sha256: artifact.sha256,
                    size_bytes: artifact.size_bytes,
                }
            })
            .collect::<Vec<_>>();
        let snapshot = qbm_domain::SourceSnapshot {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id: Sha256Digest::new("a".repeat(64)).unwrap(),
            inventory_hash: Sha256Digest::new("b".repeat(64)).unwrap(),
            policy_hash: Sha256Digest::new("c".repeat(64)).unwrap(),
            project_id: ProjectId::new("fixture").unwrap(),
            total_bytes: artifacts.iter().map(|artifact| artifact.size_bytes).sum(),
            artifacts,
            created_at: Utc::now(),
        };
        build_graph(&store, &snapshot, &GraphPolicy::default()).unwrap()
    }

    #[test]
    fn policies_find_duplicates_unresolved_cycles_and_unreachable_definitions() {
        let graph = graph_from_yaml(
            b"definitions:\n  a:\n    id: a\n    ref: b\n  b:\n    id: b\n    ref: a\n  c:\n    id: duplicate\n  d:\n    id: duplicate\n  unused:\n    id: unused\nflow:\n  ref: missing\n",
        );
        let report = audit(&graph).unwrap();
        let codes = report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect::<BTreeSet<_>>();
        assert!(codes.contains("generic.duplicate_identifier"));
        assert!(codes.contains("generic.unresolved_reference"));
        assert!(codes.contains("generic.reference_cycle"));
        assert!(codes.contains("generic.unreachable_definition"));
        assert_eq!(report.summary.total, report.findings.len() as u64);
    }

    #[test]
    fn report_identity_is_repeatable() {
        let graph = graph_from_yaml(b"id: root\nref: missing\n");
        let first = audit(&graph).unwrap();
        let second = audit(&graph).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.findings, second.findings);
    }

    #[test]
    fn duplicate_identifiers_are_scoped_to_one_explicit_definition_container() {
        let graph = graph_from_files(&[
            (
                "rules-a.yaml",
                b"rules:\n  first:\n    id: shared\nother:\n  id: shared\n",
            ),
            ("rules-b.yaml", b"rules:\n  second:\n    id: shared\n"),
        ]);
        let report = audit(&graph).unwrap();
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.code != "generic.duplicate_identifier")
        );

        let graph =
            graph_from_yaml(b"rules:\n  first:\n    id: shared\n  second:\n    id: shared\n");
        let report = audit(&graph).unwrap();
        assert_eq!(
            report
                .findings
                .iter()
                .filter(|finding| finding.code == "generic.duplicate_identifier")
                .count(),
            1
        );
    }

    #[test]
    fn iterative_scc_handles_cycle() {
        let a = Sha256Digest::new("a".repeat(64)).unwrap();
        let b = Sha256Digest::new("b".repeat(64)).unwrap();
        let evidence = EvidenceLocation {
            artifact_sha256: None,
            relative_path: None,
            pointer: None,
            line: None,
            column: None,
        };
        let edges = [
            GraphEdge {
                id: Sha256Digest::new("c".repeat(64)).unwrap(),
                from: a.clone(),
                to: Some(b.clone()),
                kind: GraphEdgeKind::References,
                reference: Some("b".to_owned()),
                evidence: evidence.clone(),
            },
            GraphEdge {
                id: Sha256Digest::new("d".repeat(64)).unwrap(),
                from: b.clone(),
                to: Some(a.clone()),
                kind: GraphEdgeKind::References,
                reference: Some("a".to_owned()),
                evidence,
            },
        ];
        let components = strongly_connected_components(&edges.iter().collect::<Vec<_>>());
        assert_eq!(components, vec![vec![a, b]]);
    }
}
