use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

use chrono::Utc;
use proc_macro2::LineColumn;
use qbm_canonical::hash_value;
use qbm_domain::{
    DOMAIN_SCHEMA_VERSION, EvidenceLocation, GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind,
    GraphPolicy, ProjectId, RawProjectGraph, ScanDiagnostic, Severity, Sha256Digest,
    SnapshotArtifact, SourceFormat, SourceSnapshot,
};
use qbm_store::PlatformStore;
use quote::ToTokens;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use syn::{Item, spanned::Spanned};

use crate::{
    AuditError,
    python::{PythonDeclarationKind, PythonScan},
};

#[derive(Debug, Serialize)]
struct NodeIdentity<'a> {
    schema_version: &'static str,
    kind: GraphNodeKind,
    label: &'a str,
    format: SourceFormat,
    identifier: Option<&'a str>,
    is_definition: bool,
    properties: &'a BTreeMap<String, Value>,
    evidence: &'a EvidenceLocation,
}

#[derive(Debug, Serialize)]
struct EdgeIdentity<'a> {
    schema_version: &'static str,
    from: &'a Sha256Digest,
    to: Option<&'a Sha256Digest>,
    kind: GraphEdgeKind,
    reference: Option<&'a str>,
    evidence: &'a EvidenceLocation,
}

#[derive(Debug, Serialize)]
struct GraphIdentity<'a> {
    schema_version: &'static str,
    snapshot_id: &'a Sha256Digest,
    project_id: &'a ProjectId,
    policy_hash: &'a Sha256Digest,
    nodes: &'a [GraphNode],
    edges: &'a [GraphEdge],
    diagnostics: &'a [ScanDiagnostic],
}

#[derive(Debug)]
struct PendingReference {
    from: Sha256Digest,
    kind: GraphEdgeKind,
    reference: String,
    evidence: EvidenceLocation,
}

struct GraphAccumulator<'a> {
    policy: &'a GraphPolicy,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    diagnostics: Vec<ScanDiagnostic>,
    pending: Vec<PendingReference>,
}

impl<'a> GraphAccumulator<'a> {
    fn new(policy: &'a GraphPolicy) -> Self {
        Self {
            policy,
            nodes: Vec::new(),
            edges: Vec::new(),
            diagnostics: Vec::new(),
            pending: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_node(
        &mut self,
        kind: GraphNodeKind,
        label: String,
        format: SourceFormat,
        identifier: Option<String>,
        is_definition: bool,
        properties: BTreeMap<String, Value>,
        evidence: EvidenceLocation,
    ) -> Result<Sha256Digest, AuditError> {
        enforce_limit(self.nodes.len(), self.policy.max_nodes, "node")?;
        let id = hash_value(&NodeIdentity {
            schema_version: "qbm.graph-node-identity/v1",
            kind,
            label: &label,
            format,
            identifier: identifier.as_deref(),
            is_definition,
            properties: &properties,
            evidence: &evidence,
        })?;
        self.nodes.push(GraphNode {
            id: id.clone(),
            kind,
            label,
            format,
            identifier,
            is_definition,
            properties,
            evidence,
        });
        Ok(id)
    }

    fn push_edge(
        &mut self,
        from: Sha256Digest,
        to: Option<Sha256Digest>,
        kind: GraphEdgeKind,
        reference: Option<String>,
        evidence: EvidenceLocation,
    ) -> Result<(), AuditError> {
        enforce_limit(self.edges.len(), self.policy.max_edges, "edge")?;
        let id = hash_value(&EdgeIdentity {
            schema_version: "qbm.graph-edge-identity/v1",
            from: &from,
            to: to.as_ref(),
            kind,
            reference: reference.as_deref(),
            evidence: &evidence,
        })?;
        self.edges.push(GraphEdge {
            id,
            from,
            to,
            kind,
            reference,
            evidence,
        });
        Ok(())
    }

    fn push_contains(
        &mut self,
        from: Sha256Digest,
        to: Sha256Digest,
        evidence: EvidenceLocation,
    ) -> Result<(), AuditError> {
        self.push_edge(from, Some(to), GraphEdgeKind::Contains, None, evidence)
    }

    fn diagnostic(
        &mut self,
        code: &str,
        severity: Severity,
        message: String,
        evidence: EvidenceLocation,
    ) {
        self.diagnostics.push(ScanDiagnostic {
            code: code.to_owned(),
            severity,
            message,
            evidence,
        });
    }
}

pub(super) fn build_graph(
    store: &PlatformStore,
    snapshot: &SourceSnapshot,
    policy: &GraphPolicy,
) -> Result<RawProjectGraph, AuditError> {
    let policy_hash = hash_value(policy)?;
    let mut graph = GraphAccumulator::new(policy);
    let project_node = graph.push_node(
        GraphNodeKind::Project,
        snapshot.project_id.to_string(),
        SourceFormat::Unknown,
        Some(snapshot.project_id.to_string()),
        false,
        BTreeMap::new(),
        synthetic_evidence(),
    )?;

    for artifact in &snapshot.artifacts {
        scan_artifact(store, artifact, &project_node, &mut graph)?;
    }
    resolve_pending_references(&mut graph)?;
    graph.nodes.sort_by(|left, right| left.id.cmp(&right.id));
    graph.edges.sort_by(|left, right| left.id.cmp(&right.id));
    graph.diagnostics.sort_by(|left, right| {
        left.evidence
            .cmp(&right.evidence)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    let id = hash_value(&GraphIdentity {
        schema_version: "qbm.raw-project-graph-identity/v1",
        snapshot_id: &snapshot.id,
        project_id: &snapshot.project_id,
        policy_hash: &policy_hash,
        nodes: &graph.nodes,
        edges: &graph.edges,
        diagnostics: &graph.diagnostics,
    })?;
    Ok(RawProjectGraph {
        schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
        id,
        snapshot_id: snapshot.id.clone(),
        project_id: snapshot.project_id.clone(),
        policy_hash,
        nodes: graph.nodes,
        edges: graph.edges,
        diagnostics: graph.diagnostics,
        created_at: Utc::now(),
    })
}

fn scan_artifact(
    store: &PlatformStore,
    artifact: &SnapshotArtifact,
    project_node: &Sha256Digest,
    graph: &mut GraphAccumulator<'_>,
) -> Result<(), AuditError> {
    let format = detect_format(&artifact.relative_path);
    let evidence = artifact_evidence(artifact, None, None);
    let mut properties = BTreeMap::new();
    properties.insert("size_bytes".to_owned(), json!(artifact.size_bytes));
    let file_node = graph.push_node(
        GraphNodeKind::File,
        artifact.relative_path.clone(),
        format,
        None,
        false,
        properties,
        evidence.clone(),
    )?;
    graph.push_contains(project_node.clone(), file_node.clone(), evidence.clone())?;
    if format == SourceFormat::Unknown {
        return Ok(());
    }
    if artifact.size_bytes > graph.policy.max_parse_file_bytes {
        graph.diagnostic(
            "parse_file_size_limit",
            Severity::Warning,
            format!(
                "supported source is {} bytes; parsing limit is {} bytes",
                artifact.size_bytes, graph.policy.max_parse_file_bytes
            ),
            evidence,
        );
        return Ok(());
    }
    let bytes = store.read_artifact(&artifact.sha256)?;
    let source = match std::str::from_utf8(&bytes) {
        Ok(source) => source,
        Err(error) => {
            graph.diagnostic(
                "invalid_utf8",
                Severity::Error,
                format!("supported text source is not UTF-8: {error}"),
                evidence,
            );
            return Ok(());
        }
    };
    match format {
        SourceFormat::Json => scan_json(source, artifact, &file_node, graph),
        SourceFormat::Yaml => scan_yaml(source, artifact, &file_node, graph),
        SourceFormat::Toml => scan_toml(source, artifact, &file_node, graph),
        SourceFormat::Rust => scan_rust(source, artifact, &file_node, graph),
        SourceFormat::Python => scan_python(source, artifact, &file_node, graph),
        SourceFormat::Unknown => Ok(()),
    }
}

fn scan_json(
    source: &str,
    artifact: &SnapshotArtifact,
    file_node: &Sha256Digest,
    graph: &mut GraphAccumulator<'_>,
) -> Result<(), AuditError> {
    match serde_json::from_str::<Value>(source) {
        Ok(value) => {
            scan_value(
                &value,
                file_node,
                "",
                "$",
                false,
                false,
                SourceFormat::Json,
                artifact,
                graph,
            )?;
        }
        Err(error) => graph.diagnostic(
            "parse_error",
            Severity::Error,
            format!("JSON parse failed: {error}"),
            artifact_evidence(
                artifact,
                None,
                Some(LineColumn {
                    line: error.line(),
                    column: error.column(),
                }),
            ),
        ),
    }
    Ok(())
}

fn scan_yaml(
    source: &str,
    artifact: &SnapshotArtifact,
    file_node: &Sha256Digest,
    graph: &mut GraphAccumulator<'_>,
) -> Result<(), AuditError> {
    let mut documents = Vec::new();
    for document in serde_yaml::Deserializer::from_str(source) {
        match Value::deserialize(document) {
            Ok(value) => documents.push(value),
            Err(error) => {
                graph.diagnostic(
                    "parse_error",
                    Severity::Error,
                    format!("YAML parse failed: {error}"),
                    artifact_evidence(artifact, None, None),
                );
                return Ok(());
            }
        }
    }
    let multiple = documents.len() > 1;
    for (index, value) in documents.iter().enumerate() {
        let pointer = if multiple {
            format!("/documents/{index}")
        } else {
            String::new()
        };
        scan_value(
            value,
            file_node,
            &pointer,
            "$",
            false,
            false,
            SourceFormat::Yaml,
            artifact,
            graph,
        )?;
    }
    Ok(())
}

fn scan_toml(
    source: &str,
    artifact: &SnapshotArtifact,
    file_node: &Sha256Digest,
    graph: &mut GraphAccumulator<'_>,
) -> Result<(), AuditError> {
    match toml::from_str::<toml::Value>(source) {
        Ok(value) => {
            let value = serde_json::to_value(value)?;
            scan_value(
                &value,
                file_node,
                "",
                "$",
                false,
                false,
                SourceFormat::Toml,
                artifact,
                graph,
            )?;
        }
        Err(error) => graph.diagnostic(
            "parse_error",
            Severity::Error,
            format!("TOML parse failed: {error}"),
            artifact_evidence(artifact, None, None),
        ),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn scan_value(
    value: &Value,
    parent: &Sha256Digest,
    pointer: &str,
    label: &str,
    is_definition: bool,
    children_are_definitions: bool,
    format: SourceFormat,
    artifact: &SnapshotArtifact,
    graph: &mut GraphAccumulator<'_>,
) -> Result<Sha256Digest, AuditError> {
    let (kind, properties) = value_shape(value);
    let identifier = value
        .as_object()
        .and_then(|map| object_identifier(map, graph.policy))
        .or_else(|| is_definition.then(|| label.to_owned()));
    let evidence = artifact_evidence(artifact, Some(pointer.to_owned()), None);
    let node = graph.push_node(
        kind,
        label.to_owned(),
        format,
        identifier,
        is_definition,
        properties,
        evidence.clone(),
    )?;
    graph.push_contains(parent.clone(), node.clone(), evidence)?;

    match value {
        Value::Object(map) => {
            collect_document_references(map, &node, pointer, artifact, graph);
            for (key, child) in map {
                let child_pointer = join_pointer(pointer, key);
                let child_is_definition = children_are_definitions;
                let child_definitions = graph.policy.definition_container_keys.contains(key);
                scan_value(
                    child,
                    &node,
                    &child_pointer,
                    key,
                    child_is_definition,
                    child_definitions,
                    format,
                    artifact,
                    graph,
                )?;
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                let label = index.to_string();
                let child_pointer = join_pointer(pointer, &label);
                scan_value(
                    child,
                    &node,
                    &child_pointer,
                    &label,
                    children_are_definitions,
                    false,
                    format,
                    artifact,
                    graph,
                )?;
            }
        }
        _ => {}
    }
    Ok(node)
}

fn value_shape(value: &Value) -> (GraphNodeKind, BTreeMap<String, Value>) {
    let mut properties = BTreeMap::new();
    let kind = match value {
        Value::Object(map) => {
            properties.insert("member_count".to_owned(), json!(map.len()));
            GraphNodeKind::Object
        }
        Value::Array(values) => {
            properties.insert("item_count".to_owned(), json!(values.len()));
            GraphNodeKind::Array
        }
        Value::Null => {
            properties.insert("value_type".to_owned(), json!("null"));
            GraphNodeKind::Scalar
        }
        Value::Bool(_) => {
            properties.insert("value_type".to_owned(), json!("boolean"));
            GraphNodeKind::Scalar
        }
        Value::Number(_) => {
            properties.insert("value_type".to_owned(), json!("number"));
            GraphNodeKind::Scalar
        }
        Value::String(_) => {
            properties.insert("value_type".to_owned(), json!("string"));
            GraphNodeKind::Scalar
        }
    };
    (kind, properties)
}

fn object_identifier(map: &Map<String, Value>, policy: &GraphPolicy) -> Option<String> {
    policy.identifier_keys.iter().find_map(|key| {
        map.get(key).and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn collect_document_references(
    map: &Map<String, Value>,
    from: &Sha256Digest,
    pointer: &str,
    artifact: &SnapshotArtifact,
    graph: &mut GraphAccumulator<'_>,
) {
    for key in &graph.policy.reference_keys {
        let Some(value) = map.get(key) else {
            continue;
        };
        for reference in reference_strings(value) {
            graph.pending.push(PendingReference {
                from: from.clone(),
                kind: GraphEdgeKind::References,
                reference,
                evidence: artifact_evidence(artifact, Some(join_pointer(pointer, key)), None),
            });
        }
    }
}

fn reference_strings(value: &Value) -> Vec<String> {
    match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn scan_rust(
    source: &str,
    artifact: &SnapshotArtifact,
    file_node: &Sha256Digest,
    graph: &mut GraphAccumulator<'_>,
) -> Result<(), AuditError> {
    let syntax = match syn::parse_file(source) {
        Ok(syntax) => syntax,
        Err(error) => {
            graph.diagnostic(
                "parse_error",
                Severity::Error,
                format!("Rust syntax parse failed: {error}"),
                artifact_evidence(artifact, None, Some(error.span().start())),
            );
            return Ok(());
        }
    };
    scan_rust_items(&syntax.items, file_node, "/rust/items", artifact, graph)
}

fn scan_rust_items(
    items: &[Item],
    parent: &Sha256Digest,
    pointer_root: &str,
    artifact: &SnapshotArtifact,
    graph: &mut GraphAccumulator<'_>,
) -> Result<(), AuditError> {
    for (index, item) in items.iter().enumerate() {
        let pointer = format!("{pointer_root}/{index}");
        let (kind, label, identifier, is_definition) = rust_item_identity(item);
        let evidence =
            artifact_evidence(artifact, Some(pointer.clone()), Some(item.span().start()));
        let node = graph.push_node(
            kind,
            label,
            SourceFormat::Rust,
            identifier,
            is_definition,
            BTreeMap::new(),
            evidence.clone(),
        )?;
        graph.push_contains(parent.clone(), node.clone(), evidence.clone())?;
        match item {
            Item::Mod(module) if module.content.is_none() => {
                graph.pending.push(PendingReference {
                    from: node,
                    kind: GraphEdgeKind::DeclaresModule,
                    reference: module.ident.to_string(),
                    evidence,
                });
            }
            Item::Mod(module) => {
                if let Some((_, nested)) = &module.content {
                    scan_rust_items(nested, &node, &format!("{pointer}/items"), artifact, graph)?;
                }
            }
            Item::Use(import) => {
                let reference = import.tree.to_token_stream().to_string().replace(' ', "");
                graph.pending.push(PendingReference {
                    from: node,
                    kind: GraphEdgeKind::Imports,
                    reference,
                    evidence,
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn rust_item_identity(item: &Item) -> (GraphNodeKind, String, Option<String>, bool) {
    let named = match item {
        Item::Const(item) => Some(("const", item.ident.to_string())),
        Item::Enum(item) => Some(("enum", item.ident.to_string())),
        Item::ExternCrate(item) => Some(("extern_crate", item.ident.to_string())),
        Item::Fn(item) => Some(("fn", item.sig.ident.to_string())),
        Item::Macro(item) => item
            .ident
            .as_ref()
            .map(|ident| ("macro", ident.to_string())),
        Item::Mod(item) => Some(("mod", item.ident.to_string())),
        Item::Static(item) => Some(("static", item.ident.to_string())),
        Item::Struct(item) => Some(("struct", item.ident.to_string())),
        Item::Trait(item) => Some(("trait", item.ident.to_string())),
        Item::TraitAlias(item) => Some(("trait_alias", item.ident.to_string())),
        Item::Type(item) => Some(("type", item.ident.to_string())),
        Item::Union(item) => Some(("union", item.ident.to_string())),
        _ => None,
    };
    if let Some((category, identifier)) = named {
        let kind = if matches!(item, Item::Mod(_)) {
            GraphNodeKind::RustModule
        } else {
            GraphNodeKind::RustItem
        };
        return (
            kind,
            format!("{category} {identifier}"),
            Some(identifier),
            true,
        );
    }
    let category = match item {
        Item::ForeignMod(_) => "extern",
        Item::Impl(_) => "impl",
        Item::Use(_) => "use",
        _ => "item",
    };
    let label = if let Item::Use(item) = item {
        format!(
            "use {}",
            item.tree.to_token_stream().to_string().replace(' ', "")
        )
    } else {
        category.to_owned()
    };
    (GraphNodeKind::RustItem, label, None, false)
}

fn scan_python(
    source: &str,
    artifact: &SnapshotArtifact,
    file_node: &Sha256Digest,
    graph: &mut GraphAccumulator<'_>,
) -> Result<(), AuditError> {
    let scan = crate::python::scan(source, &artifact.relative_path);
    for diagnostic in &scan.diagnostics {
        graph.diagnostic(
            diagnostic.code,
            if diagnostic.code == "python_token_limit" {
                Severity::Warning
            } else {
                Severity::Error
            },
            diagnostic.message.clone(),
            python_evidence(
                artifact,
                "/python/diagnostics",
                diagnostic.line,
                diagnostic.column,
            ),
        );
    }

    let mut module_properties = BTreeMap::new();
    module_properties.insert("language".to_owned(), json!("python"));
    module_properties.insert("module".to_owned(), json!(scan.module));
    module_properties.insert(
        "is_test_file".to_owned(),
        json!(python_test_file(&artifact.relative_path)),
    );
    let module_evidence = python_evidence(artifact, "/python/module", 1, 1);
    let module_node = graph.push_node(
        GraphNodeKind::PythonModule,
        format!("module {}", scan.module),
        SourceFormat::Python,
        Some(scan.module.clone()),
        true,
        module_properties,
        module_evidence.clone(),
    )?;
    graph.push_contains(file_node.clone(), module_node.clone(), module_evidence)?;

    let declaration_nodes = scan_python_declarations(&scan, artifact, &module_node, graph)?;
    scan_python_imports(&scan, artifact, &module_node, &declaration_nodes, graph);
    scan_python_calls(&scan, artifact, &module_node, &declaration_nodes, graph)
}

fn scan_python_declarations(
    scan: &PythonScan,
    artifact: &SnapshotArtifact,
    module_node: &Sha256Digest,
    graph: &mut GraphAccumulator<'_>,
) -> Result<Vec<Sha256Digest>, AuditError> {
    let mut declaration_nodes = Vec::with_capacity(scan.declarations.len());
    for (index, declaration) in scan.declarations.iter().enumerate() {
        let (kind, declaration_kind, is_test) = match declaration.kind {
            PythonDeclarationKind::Class => (GraphNodeKind::PythonClass, "class", false),
            PythonDeclarationKind::Function => (GraphNodeKind::PythonFunction, "function", false),
            PythonDeclarationKind::TestClass => (GraphNodeKind::PythonTest, "class", true),
            PythonDeclarationKind::TestFunction => (GraphNodeKind::PythonTest, "function", true),
        };
        let mut properties = BTreeMap::new();
        properties.insert("language".to_owned(), json!("python"));
        properties.insert("declaration_kind".to_owned(), json!(declaration_kind));
        properties.insert("simple_name".to_owned(), json!(declaration.name));
        properties.insert(
            "qualified_name".to_owned(),
            json!(declaration.qualified_name),
        );
        properties.insert("is_async".to_owned(), json!(declaration.is_async));
        properties.insert("is_test".to_owned(), json!(is_test));
        let evidence = python_evidence(
            artifact,
            &format!("/python/declarations/{index}"),
            declaration.line,
            declaration.column,
        );
        let node = graph.push_node(
            kind,
            format!("{declaration_kind} {}", declaration.name),
            SourceFormat::Python,
            Some(declaration.qualified_name.clone()),
            true,
            properties,
            evidence.clone(),
        )?;
        let parent = declaration.parent.map_or(module_node, |parent| {
            // The scanner only emits a parent after its declaration.
            &declaration_nodes[parent]
        });
        graph.push_contains(parent.clone(), node.clone(), evidence)?;
        declaration_nodes.push(node);
    }
    Ok(declaration_nodes)
}

fn scan_python_imports(
    scan: &PythonScan,
    artifact: &SnapshotArtifact,
    module_node: &Sha256Digest,
    declarations: &[Sha256Digest],
    graph: &mut GraphAccumulator<'_>,
) {
    for (index, import) in scan.imports.iter().enumerate() {
        graph.pending.push(PendingReference {
            from: import
                .owner
                .map_or_else(|| module_node.clone(), |owner| declarations[owner].clone()),
            kind: GraphEdgeKind::Imports,
            reference: import.reference.clone(),
            evidence: python_evidence(
                artifact,
                &format!("/python/imports/{index}"),
                import.line,
                import.column,
            ),
        });
    }
}

fn scan_python_calls(
    scan: &PythonScan,
    artifact: &SnapshotArtifact,
    module_node: &Sha256Digest,
    declarations: &[Sha256Digest],
    graph: &mut GraphAccumulator<'_>,
) -> Result<(), AuditError> {
    for (index, call) in scan.calls.iter().enumerate() {
        let mut properties = BTreeMap::new();
        properties.insert("language".to_owned(), json!("python"));
        properties.insert("reference".to_owned(), json!(call.reference));
        properties.insert(
            "expanded_reference".to_owned(),
            json!(call.expanded_reference),
        );
        let evidence = python_evidence(
            artifact,
            &format!("/python/calls/{index}"),
            call.line,
            call.column,
        );
        let call_node = graph.push_node(
            GraphNodeKind::PythonCall,
            format!("call {}", call.reference),
            SourceFormat::Python,
            None,
            false,
            properties,
            evidence.clone(),
        )?;
        let parent = call
            .owner
            .map_or_else(|| module_node.clone(), |owner| declarations[owner].clone());
        graph.push_contains(parent, call_node.clone(), evidence.clone())?;
        graph.pending.push(PendingReference {
            from: call_node,
            kind: GraphEdgeKind::Calls,
            reference: call.expanded_reference.clone(),
            evidence,
        });
    }
    Ok(())
}

fn python_evidence(
    artifact: &SnapshotArtifact,
    pointer: &str,
    line: u64,
    column: u64,
) -> EvidenceLocation {
    EvidenceLocation {
        artifact_sha256: Some(artifact.sha256.clone()),
        relative_path: Some(artifact.relative_path.clone()),
        pointer: Some(pointer.to_owned()),
        line: Some(line),
        column: Some(column),
    }
}

fn python_test_file(path: &str) -> bool {
    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    file_name.starts_with("test_")
        || file_name.ends_with("_test.py")
        || path.split('/').any(|part| matches!(part, "test" | "tests"))
}

fn resolve_pending_references(graph: &mut GraphAccumulator<'_>) -> Result<(), AuditError> {
    let mut identifiers: BTreeMap<String, Vec<Sha256Digest>> = BTreeMap::new();
    let mut pointers: BTreeMap<(String, String), Sha256Digest> = BTreeMap::new();
    let mut files: BTreeMap<String, Sha256Digest> = BTreeMap::new();
    for node in &graph.nodes {
        if let Some(identifier) = &node.identifier {
            identifiers
                .entry(identifier.clone())
                .or_default()
                .push(node.id.clone());
        }
        if let Some(simple_name) = node.properties.get("simple_name").and_then(Value::as_str) {
            identifiers
                .entry(simple_name.to_owned())
                .or_default()
                .push(node.id.clone());
        }
        if let (Some(path), Some(pointer)) = (&node.evidence.relative_path, &node.evidence.pointer)
        {
            pointers.insert((path.clone(), pointer.clone()), node.id.clone());
        }
        if node.kind == GraphNodeKind::File {
            if let Some(path) = &node.evidence.relative_path {
                files.insert(path.clone(), node.id.clone());
            }
        }
    }
    let pending = std::mem::take(&mut graph.pending);
    for reference in pending {
        let target = resolve_one(&reference, &identifiers, &pointers, &files);
        graph.push_edge(
            reference.from,
            target,
            reference.kind,
            Some(reference.reference),
            reference.evidence,
        )?;
    }
    Ok(())
}

fn resolve_one(
    pending: &PendingReference,
    identifiers: &BTreeMap<String, Vec<Sha256Digest>>,
    pointers: &BTreeMap<(String, String), Sha256Digest>,
    files: &BTreeMap<String, Sha256Digest>,
) -> Option<Sha256Digest> {
    let source_path = pending.evidence.relative_path.as_deref()?;
    match pending.kind {
        GraphEdgeKind::DeclaresModule => {
            let parent = Path::new(source_path)
                .parent()
                .unwrap_or_else(|| Path::new(""));
            let direct = parent.join(format!("{}.rs", pending.reference));
            let nested = parent.join(&pending.reference).join("mod.rs");
            direct
                .to_str()
                .and_then(|path| files.get(path).cloned())
                .or_else(|| nested.to_str().and_then(|path| files.get(path).cloned()))
        }
        GraphEdgeKind::Imports | GraphEdgeKind::Calls => {
            unique_identifier(identifiers, &pending.reference).or_else(|| {
                let identifier = pending
                    .reference
                    .rsplit([':', '.'])
                    .find(|part| !matches!(*part, "" | "self" | "super" | "crate" | "*"))?;
                unique_identifier(identifiers, identifier)
            })
        }
        GraphEdgeKind::References => {
            resolve_document_reference(source_path, &pending.reference, identifiers, pointers)
        }
        GraphEdgeKind::Contains => None,
    }
}

fn resolve_document_reference(
    source_path: &str,
    reference: &str,
    identifiers: &BTreeMap<String, Vec<Sha256Digest>>,
    pointers: &BTreeMap<(String, String), Sha256Digest>,
) -> Option<Sha256Digest> {
    if let Some(pointer) = reference.strip_prefix('#') {
        let direct = pointers
            .get(&(source_path.to_owned(), pointer.to_owned()))
            .cloned();
        return direct.or_else(|| {
            let matches = pointers
                .iter()
                .filter(|((path, candidate), _)| {
                    path == source_path && yaml_document_pointer_matches(candidate, pointer)
                })
                .map(|(_, id)| id)
                .collect::<Vec<_>>();
            (matches.len() == 1).then(|| matches[0].clone())
        });
    }
    if let Some((path, pointer)) = reference.split_once('#') {
        let path = resolve_relative_reference_path(source_path, path)?;
        return pointers.get(&(path, pointer.to_owned())).cloned();
    }
    unique_identifier(identifiers, reference)
}

fn yaml_document_pointer_matches(candidate: &str, pointer: &str) -> bool {
    let Some(prefix) = candidate.strip_suffix(pointer) else {
        return false;
    };
    let Some(index) = prefix.strip_prefix("/documents/") else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

fn unique_identifier(
    identifiers: &BTreeMap<String, Vec<Sha256Digest>>,
    identifier: &str,
) -> Option<Sha256Digest> {
    let candidates = identifiers.get(identifier)?;
    (candidates.len() == 1).then(|| candidates[0].clone())
}

fn resolve_relative_reference_path(source_path: &str, reference: &str) -> Option<String> {
    let parent = Path::new(source_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let combined = parent.join(reference);
    let mut parts = Vec::new();
    for component in combined.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_str()?.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
}

fn detect_format(path: &str) -> SourceFormat {
    let extension = Path::new(path).extension().and_then(|value| value.to_str());
    if extension.is_some_and(|value| value.eq_ignore_ascii_case("json")) {
        SourceFormat::Json
    } else if extension.is_some_and(|value| {
        value.eq_ignore_ascii_case("yaml") || value.eq_ignore_ascii_case("yml")
    }) {
        SourceFormat::Yaml
    } else if extension.is_some_and(|value| value.eq_ignore_ascii_case("toml")) {
        SourceFormat::Toml
    } else if extension.is_some_and(|value| value.eq_ignore_ascii_case("rs")) {
        SourceFormat::Rust
    } else if extension.is_some_and(|value| value.eq_ignore_ascii_case("py")) {
        SourceFormat::Python
    } else {
        SourceFormat::Unknown
    }
}

fn artifact_evidence(
    artifact: &SnapshotArtifact,
    pointer: Option<String>,
    position: Option<LineColumn>,
) -> EvidenceLocation {
    EvidenceLocation {
        artifact_sha256: Some(artifact.sha256.clone()),
        relative_path: Some(artifact.relative_path.clone()),
        pointer,
        line: position.map(|value| value.line as u64),
        column: position.map(|value| value.column as u64 + 1),
    }
}

fn synthetic_evidence() -> EvidenceLocation {
    EvidenceLocation {
        artifact_sha256: None,
        relative_path: None,
        pointer: None,
        line: None,
        column: None,
    }
}

fn join_pointer(parent: &str, key: &str) -> String {
    let escaped = key.replace('~', "~0").replace('/', "~1");
    format!("{parent}/{escaped}")
}

fn enforce_limit(observed: usize, maximum: u64, kind: &'static str) -> Result<(), AuditError> {
    if u64::try_from(observed).unwrap_or(u64::MAX) >= maximum {
        Err(AuditError::GraphLimit { kind, maximum })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use qbm_domain::{ProjectId, SnapshotArtifact};
    use tempfile::TempDir;

    use super::*;

    fn snapshot_for(store: &PlatformStore, files: &[(&str, &[u8])]) -> SourceSnapshot {
        let artifacts = files
            .iter()
            .map(|(path, bytes)| {
                let artifact = store
                    .put_bytes(bytes, Some((*path).to_owned()), None)
                    .unwrap();
                SnapshotArtifact {
                    relative_path: (*path).to_owned(),
                    sha256: artifact.sha256,
                    size_bytes: artifact.size_bytes,
                }
            })
            .collect();
        SourceSnapshot {
            schema_version: DOMAIN_SCHEMA_VERSION.to_owned(),
            id: Sha256Digest::new("a".repeat(64)).unwrap(),
            inventory_hash: Sha256Digest::new("b".repeat(64)).unwrap(),
            policy_hash: Sha256Digest::new("c".repeat(64)).unwrap(),
            project_id: ProjectId::new("fixture").unwrap(),
            artifacts,
            total_bytes: files.iter().map(|(_, bytes)| bytes.len() as u64).sum(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn supported_documents_and_rust_build_one_deterministic_graph() {
        let data = TempDir::new().unwrap();
        let store = PlatformStore::open(data.path()).unwrap();
        let snapshot = snapshot_for(
            &store,
            &[
                (
                    "rules.yaml",
                    b"definitions:\n  anemia:\n    id: anemia\nflow:\n  ref: anemia\n",
                ),
                (
                    "config.json",
                    br##"{"$ref":"#/items/0","items":[{"id":"x"}]}"##,
                ),
                ("settings.toml", b"id = \"settings\"\n"),
                ("src/lib.rs", b"mod engine;\npub fn run() {}\n"),
                ("src/engine.rs", b"pub struct Engine;\n"),
            ],
        );
        let first = build_graph(&store, &snapshot, &GraphPolicy::default()).unwrap();
        let second = build_graph(&store, &snapshot, &GraphPolicy::default()).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.diagnostics.len(), 0);
        assert!(
            first
                .nodes
                .iter()
                .any(|node| node.identifier.as_deref() == Some("anemia"))
        );
        assert!(
            first
                .edges
                .iter()
                .any(|edge| { edge.kind == GraphEdgeKind::DeclaresModule && edge.to.is_some() })
        );
    }

    #[test]
    fn python_builds_evidence_linked_symbols_imports_calls_and_tests() {
        let data = TempDir::new().unwrap();
        let store = PlatformStore::open(data.path()).unwrap();
        let snapshot = snapshot_for(
            &store,
            &[
                (
                    "pkg/helpers.py",
                    b"def normalize(value):\n    return value\n",
                ),
                (
                    "pkg/engine.py",
                    b"from .helpers import normalize as clean\n\nclass Pipeline:\n    def run(self, value):\n        def finish(item):\n            return clean(item)\n        return finish(value)\n",
                ),
                (
                    "tests/test_engine.py",
                    b"from pkg.engine import Pipeline\n\ndef test_pipeline():\n    Pipeline().run(1)\n",
                ),
            ],
        );

        let first = build_graph(&store, &snapshot, &GraphPolicy::default()).unwrap();
        let second = build_graph(&store, &snapshot, &GraphPolicy::default()).unwrap();
        assert_eq!(first.id, second.id);
        assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
        assert!(first.nodes.iter().any(|node| {
            node.kind == GraphNodeKind::PythonModule
                && node.identifier.as_deref() == Some("pkg.engine")
        }));
        assert!(first.nodes.iter().any(|node| {
            node.kind == GraphNodeKind::PythonFunction
                && node.identifier.as_deref() == Some("pkg.engine.Pipeline.run.finish")
                && node.evidence.relative_path.as_deref() == Some("pkg/engine.py")
                && node.evidence.line == Some(5)
        }));
        assert!(first.nodes.iter().any(|node| {
            node.kind == GraphNodeKind::PythonTest
                && node.identifier.as_deref() == Some("tests.test_engine.test_pipeline")
                && node.properties.get("is_test") == Some(&json!(true))
        }));
        assert!(first.edges.iter().any(|edge| {
            edge.kind == GraphEdgeKind::Imports
                && edge.reference.as_deref() == Some("pkg.helpers.normalize")
                && edge.to.is_some()
        }));
        assert!(first.edges.iter().any(|edge| {
            edge.kind == GraphEdgeKind::Calls
                && edge.reference.as_deref() == Some("pkg.helpers.normalize")
                && edge.to.is_some()
                && edge.evidence.line == Some(6)
        }));
    }

    #[test]
    fn malformed_python_is_diagnostic_and_never_executes_source() {
        let data = TempDir::new().unwrap();
        let store = PlatformStore::open(data.path()).unwrap();
        let marker = data.path().join("must-not-exist");
        let source = format!(
            "import os\nos.system('touch {}')\ndef broken(:\n",
            marker.display()
        );
        let snapshot = snapshot_for(&store, &[("unsafe.py", source.as_bytes())]);

        let graph = build_graph(&store, &snapshot, &GraphPolicy::default()).unwrap();
        assert!(!marker.exists());
        assert!(
            graph
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "python_syntax_error")
        );
        assert!(graph.nodes.iter().any(|node| {
            node.kind == GraphNodeKind::PythonCall
                && node.properties.get("reference") == Some(&json!("os.system"))
        }));
    }

    #[test]
    fn parse_failures_are_diagnostics_not_partial_scan_failures() {
        let data = TempDir::new().unwrap();
        let store = PlatformStore::open(data.path()).unwrap();
        let snapshot = snapshot_for(&store, &[("broken.json", b"{not-json")]);
        let graph = build_graph(&store, &snapshot, &GraphPolicy::default()).unwrap();
        assert_eq!(graph.diagnostics.len(), 1);
        assert_eq!(graph.diagnostics[0].code, "parse_error");
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == GraphNodeKind::File)
        );
    }

    #[test]
    fn parser_never_writes_to_source_directories() {
        let data = TempDir::new().unwrap();
        let source = TempDir::new().unwrap();
        fs::write(source.path().join("marker"), b"unchanged").unwrap();
        let store = PlatformStore::open(data.path()).unwrap();
        let snapshot = snapshot_for(&store, &[("input.yaml", b"id: safe\n")]);
        build_graph(&store, &snapshot, &GraphPolicy::default()).unwrap();
        assert_eq!(
            fs::read(source.path().join("marker")).unwrap(),
            b"unchanged"
        );
    }
}
