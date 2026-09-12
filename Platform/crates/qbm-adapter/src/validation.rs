use std::collections::{BTreeMap, BTreeSet};

use qbm_domain::{
    ADAPTER_API_VERSION, ADAPTER_PACKAGE_SCHEMA_VERSION, AdapterCapability,
    AdapterConformanceIssue, AdapterPackage, MappingValueSource, RawNodeSelector,
    SemanticPolicyAssertion, Severity,
};

use crate::AdapterLimits;

pub(super) const MANIFEST_SCHEMA_VERSION: &str = "qbm.adapter-manifest/v1";
pub(super) const MAPPING_SCHEMA_VERSION: &str = "qbm.mapping-pack/v1";
pub(super) const POLICY_SCHEMA_VERSION: &str = "qbm.semantic-policy-pack/v1";

pub(super) struct ValidationResult {
    pub checks: u64,
    pub issues: Vec<AdapterConformanceIssue>,
}

struct Validator<'a> {
    package: &'a AdapterPackage,
    limits: AdapterLimits,
    checks: u64,
    issues: Vec<AdapterConformanceIssue>,
}

impl Validator<'_> {
    fn check(&mut self, condition: bool, code: &str, message: impl Into<String>) {
        self.check_fixture(condition, code, message, None);
    }

    fn check_fixture(
        &mut self,
        condition: bool,
        code: &str,
        message: impl Into<String>,
        fixture_id: Option<&str>,
    ) {
        self.checks = self.checks.saturating_add(1);
        if !condition {
            self.issues.push(AdapterConformanceIssue {
                code: code.to_owned(),
                severity: Severity::Error,
                message: message.into(),
                fixture_id: fixture_id.map(str::to_owned),
            });
        }
    }

    fn validate(mut self) -> ValidationResult {
        self.schemas_and_manifest();
        self.resource_counts();
        self.detection();
        self.mappings();
        self.policies();
        self.projections();
        self.fixtures();
        ValidationResult {
            checks: self.checks,
            issues: self.issues,
        }
    }

    fn schemas_and_manifest(&mut self) {
        let manifest = &self.package.manifest;
        self.check(
            self.package.schema_version == ADAPTER_PACKAGE_SCHEMA_VERSION,
            "package_schema",
            format!(
                "package schema must be {ADAPTER_PACKAGE_SCHEMA_VERSION:?}, got {:?}",
                self.package.schema_version
            ),
        );
        self.check(
            manifest.schema_version == MANIFEST_SCHEMA_VERSION,
            "manifest_schema",
            format!(
                "manifest schema must be {MANIFEST_SCHEMA_VERSION:?}, got {:?}",
                manifest.schema_version
            ),
        );
        self.check(
            manifest.adapter_api_version == ADAPTER_API_VERSION,
            "adapter_api_version",
            format!(
                "adapter API must be {ADAPTER_API_VERSION:?}, got {:?}",
                manifest.adapter_api_version
            ),
        );
        self.check(
            manifest.minimum_platform_schema == qbm_domain::DOMAIN_SCHEMA_VERSION,
            "minimum_platform_schema",
            format!(
                "this interpreter requires minimum_platform_schema {:?}",
                qbm_domain::DOMAIN_SCHEMA_VERSION
            ),
        );
        self.check(
            qbm_domain::AdapterId::new(manifest.id.to_string()).is_ok(),
            "adapter_id",
            "adapter ID violates the portable identifier policy",
        );
        self.check(
            valid_short(&manifest.version, 64),
            "adapter_version",
            "adapter version must be 1-64 printable non-whitespace characters",
        );
        self.check(
            valid_text(&manifest.display_name, 128),
            "display_name",
            "adapter display name must be 1-128 trimmed characters",
        );
        self.check(
            valid_text(&manifest.description, 2_048),
            "description",
            "adapter description must be 1-2048 trimmed characters",
        );
        self.check(
            valid_short(&manifest.license, 128),
            "license",
            "adapter license must be 1-128 printable non-whitespace characters",
        );
        self.check(
            manifest
                .capabilities
                .contains(&AdapterCapability::RawGraphRead),
            "raw_graph_capability",
            "every adapter must declare raw_graph_read",
        );
    }

    fn resource_counts(&mut self) {
        self.check(
            self.package.detection.len() <= self.limits.max_detection_rules,
            "detection_rule_limit",
            format!(
                "package has {} detection rules; maximum is {}",
                self.package.detection.len(),
                self.limits.max_detection_rules
            ),
        );
        self.check(
            self.package
                .mappings
                .nodes
                .len()
                .saturating_add(self.package.mappings.edges.len())
                <= self.limits.max_mapping_rules,
            "mapping_rule_limit",
            format!(
                "package has too many mapping rules; maximum is {}",
                self.limits.max_mapping_rules
            ),
        );
        self.check(
            self.package.policies.rules.len() <= self.limits.max_policy_rules,
            "policy_rule_limit",
            format!(
                "package has too many policy rules; maximum is {}",
                self.limits.max_policy_rules
            ),
        );
        self.check(
            self.package.projections.len() <= self.limits.max_projections,
            "projection_limit",
            format!(
                "package has too many projections; maximum is {}",
                self.limits.max_projections
            ),
        );
        self.check(
            self.package.fixtures.len() <= self.limits.max_fixtures,
            "fixture_limit",
            format!(
                "package has too many fixtures; maximum is {}",
                self.limits.max_fixtures
            ),
        );
        let fixture_nodes = self
            .package
            .fixtures
            .iter()
            .fold(0_usize, |total, fixture| {
                total.saturating_add(fixture.nodes.len())
            });
        self.check(
            fixture_nodes <= self.limits.max_fixture_nodes,
            "fixture_node_limit",
            format!(
                "package has {fixture_nodes} fixture nodes; maximum is {}",
                self.limits.max_fixture_nodes
            ),
        );
        let fixture_edges = self
            .package
            .fixtures
            .iter()
            .fold(0_usize, |total, fixture| {
                total.saturating_add(fixture.edges.len())
            });
        self.check(
            fixture_edges <= self.limits.max_fixture_edges,
            "fixture_edge_limit",
            format!(
                "package has {fixture_edges} fixture edges; maximum is {}",
                self.limits.max_fixture_edges
            ),
        );
    }

    fn detection(&mut self) {
        self.check(
            !self.package.detection.is_empty(),
            "detection_empty",
            "at least one detection rule is required",
        );
        self.check(
            self.package.detection.iter().any(|rule| rule.required),
            "required_detection_rule_missing",
            "at least one detection rule must be required",
        );
        let mut ids = BTreeSet::new();
        for rule in &self.package.detection {
            self.check(
                portable_name(&rule.id, 64),
                "detection_rule_id",
                format!("detection rule ID {:?} is not portable", rule.id),
            );
            self.check(
                ids.insert(rule.id.clone()),
                "duplicate_detection_rule_id",
                format!("duplicate detection rule ID {:?}", rule.id),
            );
            self.check(
                valid_text(&rule.description, 2_048),
                "detection_rule_description",
                format!("detection rule {:?} has an invalid description", rule.id),
            );
            self.check(
                rule.minimum_matches > 0,
                "detection_minimum",
                format!(
                    "detection rule {:?} must require at least one match",
                    rule.id
                ),
            );
            self.check(
                rule.weight > 0,
                "detection_weight",
                format!("detection rule {:?} must have a positive weight", rule.id),
            );
            self.validate_selector(&rule.selector, "detection_selector", &rule.id);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn mappings(&mut self) {
        self.check(
            self.package.mappings.schema_version == MAPPING_SCHEMA_VERSION,
            "mapping_schema",
            format!("mapping schema must be {MAPPING_SCHEMA_VERSION:?}"),
        );
        self.check(
            !self.package.mappings.nodes.is_empty(),
            "node_mapping_empty",
            "at least one node mapping is required",
        );
        self.check(
            self.package
                .manifest
                .capabilities
                .contains(&AdapterCapability::EmitSemanticNodes),
            "semantic_node_capability",
            "node mappings require emit_semantic_nodes",
        );
        self.check(
            self.package.mappings.edges.is_empty()
                || self
                    .package
                    .manifest
                    .capabilities
                    .contains(&AdapterCapability::EmitSemanticEdges),
            "semantic_edge_capability",
            "edge mappings require emit_semantic_edges",
        );
        let mut ids = BTreeSet::new();
        for rule in &self.package.mappings.nodes {
            self.check(
                rule.attributes.len() <= self.limits.max_attributes_per_rule,
                "mapping_attribute_limit",
                format!(
                    "node mapping {:?} has {} attributes; maximum is {}",
                    rule.id,
                    rule.attributes.len(),
                    self.limits.max_attributes_per_rule
                ),
            );
            self.check(
                portable_name(&rule.id, 64),
                "node_mapping_id",
                format!("node mapping ID {:?} is not portable", rule.id),
            );
            self.check(
                ids.insert(rule.id.clone()),
                "duplicate_mapping_rule_id",
                format!("duplicate mapping rule ID {:?}", rule.id),
            );
            self.check(
                self.namespaced(&rule.semantic_type),
                "semantic_type_namespace",
                format!(
                    "semantic type {:?} is outside the adapter namespace",
                    rule.semantic_type
                ),
            );
            self.validate_selector(&rule.selector, "mapping_selector", &rule.id);
            let mut attributes = BTreeSet::new();
            for attribute in &rule.attributes {
                self.check(
                    portable_name(&attribute.target, 64),
                    "attribute_name",
                    format!("attribute {:?} is not portable", attribute.target),
                );
                self.check(
                    attributes.insert(attribute.target.clone()),
                    "duplicate_attribute",
                    format!(
                        "node mapping {:?} declares attribute {:?} more than once",
                        rule.id, attribute.target
                    ),
                );
                if let MappingValueSource::RawProperty { key } = &attribute.value {
                    self.check(
                        portable_name(key, 64),
                        "raw_property_name",
                        format!("raw property key {key:?} is not portable"),
                    );
                }
                if matches!(attribute.value, MappingValueSource::EvidenceValue) {
                    self.check(
                        self.package
                            .manifest
                            .capabilities
                            .contains(&AdapterCapability::SnapshotArtifactTextRead),
                        "evidence_value_capability",
                        format!(
                            "node mapping {:?} uses evidence_value without snapshot_artifact_text_read",
                            rule.id
                        ),
                    );
                }
                if let MappingValueSource::Literal { value } = &attribute.value {
                    let within_limit = qbm_canonical::canonical_json(value)
                        .is_ok_and(|bytes| bytes.len() <= self.limits.max_attribute_bytes);
                    self.check(
                        within_limit,
                        "literal_value_limit",
                        format!(
                            "node mapping {:?} literal for {:?} exceeds {} canonical bytes",
                            rule.id, attribute.target, self.limits.max_attribute_bytes
                        ),
                    );
                }
            }
        }
        for rule in &self.package.mappings.edges {
            self.check(
                portable_name(&rule.id, 64),
                "edge_mapping_id",
                format!("edge mapping ID {:?} is not portable", rule.id),
            );
            self.check(
                ids.insert(rule.id.clone()),
                "duplicate_mapping_rule_id",
                format!("duplicate mapping rule ID {:?}", rule.id),
            );
            self.check(
                self.namespaced(&rule.relation_type),
                "relation_type_namespace",
                format!(
                    "relationship type {:?} is outside the adapter namespace",
                    rule.relation_type
                ),
            );
            for semantic_type in [
                rule.from_semantic_type.as_deref(),
                rule.to_semantic_type.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                self.check(
                    self.namespaced(semantic_type),
                    "edge_semantic_type_namespace",
                    format!("edge filter type {semantic_type:?} is outside the adapter namespace"),
                );
                self.check(
                    self.package
                        .mappings
                        .nodes
                        .iter()
                        .any(|mapping| mapping.semantic_type == semantic_type),
                    "edge_semantic_type_unknown",
                    format!("edge filter type {semantic_type:?} is not emitted by a node mapping"),
                );
            }
        }
    }

    fn policies(&mut self) {
        self.check(
            self.package.policies.schema_version == POLICY_SCHEMA_VERSION,
            "policy_schema",
            format!("policy schema must be {POLICY_SCHEMA_VERSION:?}"),
        );
        self.check(
            self.package.policies.rules.is_empty()
                || self
                    .package
                    .manifest
                    .capabilities
                    .contains(&AdapterCapability::EmitFindings),
            "finding_capability",
            "semantic policy rules require emit_findings",
        );
        let mut ids = BTreeSet::new();
        for rule in &self.package.policies.rules {
            self.check(
                portable_name(&rule.id, 64),
                "policy_rule_id",
                format!("policy rule ID {:?} is not portable", rule.id),
            );
            self.check(
                ids.insert(rule.id.clone()),
                "duplicate_policy_rule_id",
                format!("duplicate policy rule ID {:?}", rule.id),
            );
            self.check(
                valid_text(&rule.title, 256)
                    && valid_text(&rule.description, 2_048)
                    && valid_text(&rule.remediation, 2_048),
                "policy_text",
                format!("policy rule {:?} has empty or oversized text", rule.id),
            );
            match &rule.assertion {
                SemanticPolicyAssertion::MinimumTypeCount {
                    semantic_type,
                    minimum,
                } => {
                    self.validate_policy_type(semantic_type, &rule.id);
                    self.check(
                        *minimum > 0,
                        "policy_minimum",
                        format!("policy rule {:?} must use a positive minimum", rule.id),
                    );
                }
                SemanticPolicyAssertion::MaximumTypeCount { semantic_type, .. }
                | SemanticPolicyAssertion::UniqueExternalId { semantic_type } => {
                    self.validate_policy_type(semantic_type, &rule.id);
                }
                SemanticPolicyAssertion::RequiredAttribute {
                    semantic_type,
                    attribute,
                } => {
                    self.validate_policy_type(semantic_type, &rule.id);
                    self.check(
                        portable_name(attribute, 64),
                        "policy_attribute",
                        format!(
                            "policy rule {:?} has invalid attribute {attribute:?}",
                            rule.id
                        ),
                    );
                    self.check(
                        self.package.mappings.nodes.iter().any(|mapping| {
                            mapping.semantic_type == *semantic_type
                                && mapping
                                    .attributes
                                    .iter()
                                    .any(|binding| binding.target == *attribute)
                        }),
                        "policy_attribute_unknown",
                        format!(
                            "policy rule {:?} requires attribute {attribute:?} not emitted for {semantic_type:?}",
                            rule.id
                        ),
                    );
                }
                SemanticPolicyAssertion::RequiredOutgoingRelation {
                    semantic_type,
                    relation_type,
                } => {
                    self.validate_policy_type(semantic_type, &rule.id);
                    self.validate_policy_relation(relation_type, &rule.id);
                }
                SemanticPolicyAssertion::ForbidSelfRelation { relation_type } => {
                    self.validate_policy_relation(relation_type, &rule.id);
                }
            }
        }
    }

    fn projections(&mut self) {
        self.check(
            self.package.projections.is_empty()
                || self
                    .package
                    .manifest
                    .capabilities
                    .contains(&AdapterCapability::EmitProjections),
            "projection_capability",
            "projection declarations require emit_projections",
        );
        let mut ids = BTreeSet::new();
        for projection in &self.package.projections {
            self.check(
                portable_name(&projection.id, 64),
                "projection_id",
                format!("projection ID {:?} is not portable", projection.id),
            );
            self.check(
                ids.insert(projection.id.clone()),
                "duplicate_projection_id",
                format!("duplicate projection ID {:?}", projection.id),
            );
            self.check(
                valid_text(&projection.display_name, 128)
                    && valid_text(&projection.description, 2_048),
                "projection_text",
                format!("projection {:?} has empty or oversized text", projection.id),
            );
            for semantic_type in &projection.semantic_types {
                self.check(
                    self.namespaced(semantic_type),
                    "projection_type_namespace",
                    format!("projection type {semantic_type:?} is outside the adapter namespace"),
                );
                self.check(
                    self.package
                        .mappings
                        .nodes
                        .iter()
                        .any(|mapping| mapping.semantic_type == *semantic_type),
                    "projection_type_unknown",
                    format!("projection type {semantic_type:?} is not emitted by a node mapping"),
                );
            }
            for relation_type in &projection.relation_types {
                self.check(
                    self.namespaced(relation_type),
                    "projection_relation_namespace",
                    format!(
                        "projection relationship {relation_type:?} is outside the adapter namespace"
                    ),
                );
                self.check(
                    self.package
                        .mappings
                        .edges
                        .iter()
                        .any(|mapping| mapping.relation_type == *relation_type),
                    "projection_relation_unknown",
                    format!(
                        "projection relationship {relation_type:?} is not emitted by an edge mapping"
                    ),
                );
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn fixtures(&mut self) {
        self.check(
            !self.package.fixtures.is_empty(),
            "fixtures_empty",
            "at least one conformance fixture is required",
        );
        let mut ids = BTreeSet::new();
        let has_positive = self
            .package
            .fixtures
            .iter()
            .any(|fixture| fixture.expect.detection_eligible);
        self.check(
            has_positive,
            "positive_fixture_missing",
            "at least one fixture must expect detection eligibility",
        );
        let has_negative = self
            .package
            .fixtures
            .iter()
            .any(|fixture| !fixture.expect.detection_eligible);
        self.check(
            has_negative,
            "negative_fixture_missing",
            "at least one fixture must expect detection ineligibility",
        );
        for fixture in &self.package.fixtures {
            self.check_fixture(
                portable_name(&fixture.id, 64),
                "fixture_id",
                format!("fixture ID {:?} is not portable", fixture.id),
                Some(&fixture.id),
            );
            self.check_fixture(
                ids.insert(fixture.id.clone()),
                "duplicate_fixture_id",
                format!("duplicate fixture ID {:?}", fixture.id),
                Some(&fixture.id),
            );
            let mut node_keys = BTreeSet::new();
            for node in &fixture.nodes {
                self.check_fixture(
                    portable_name(&node.key, 64),
                    "fixture_node_key",
                    format!("fixture node key {:?} is not portable", node.key),
                    Some(&fixture.id),
                );
                self.check_fixture(
                    node_keys.insert(node.key.clone()),
                    "duplicate_fixture_node_key",
                    format!("duplicate fixture node key {:?}", node.key),
                    Some(&fixture.id),
                );
                self.check_fixture(
                    safe_relative_path(&node.relative_path),
                    "fixture_path",
                    format!(
                        "fixture path {:?} is not a safe relative path",
                        node.relative_path
                    ),
                    Some(&fixture.id),
                );
            }
            for edge in &fixture.edges {
                self.check_fixture(
                    node_keys.contains(&edge.from),
                    "fixture_edge_source",
                    format!("fixture edge source {:?} does not exist", edge.from),
                    Some(&fixture.id),
                );
                if let Some(target) = &edge.to {
                    self.check_fixture(
                        node_keys.contains(target),
                        "fixture_edge_target",
                        format!("fixture edge target {target:?} does not exist"),
                        Some(&fixture.id),
                    );
                }
            }
            self.check_fixture(
                fixture.expect.minimum_confidence_bps <= 10_000,
                "fixture_confidence",
                "fixture confidence must be between 0 and 10000 basis points",
                Some(&fixture.id),
            );
            for semantic_type in fixture.expect.semantic_type_counts.keys() {
                self.check_fixture(
                    self.namespaced(semantic_type),
                    "fixture_semantic_type_namespace",
                    format!("fixture semantic type {semantic_type:?} is outside the namespace"),
                    Some(&fixture.id),
                );
            }
            for relation_type in fixture.expect.semantic_relation_counts.keys() {
                self.check_fixture(
                    self.namespaced(relation_type),
                    "fixture_relation_type_namespace",
                    format!("fixture relationship {relation_type:?} is outside the namespace"),
                    Some(&fixture.id),
                );
            }
            for finding_code in &fixture.expect.finding_codes {
                let expected_prefix = format!("{}::", self.package.manifest.id);
                self.check_fixture(
                    finding_code.starts_with(&expected_prefix),
                    "fixture_finding_namespace",
                    format!("fixture finding code {finding_code:?} is outside the namespace"),
                    Some(&fixture.id),
                );
            }
            for projection_id in fixture.expect.projection_availability.keys() {
                self.check_fixture(
                    self.package
                        .projections
                        .iter()
                        .any(|projection| projection.id == *projection_id),
                    "fixture_projection_unknown",
                    format!("fixture projection ID {projection_id:?} is not declared"),
                    Some(&fixture.id),
                );
            }
        }
    }

    fn validate_selector(&mut self, selector: &RawNodeSelector, code: &str, rule_id: &str) {
        for path in selector.path_prefixes.iter().chain(&selector.path_suffixes) {
            self.check(
                safe_path_fragment(path),
                code,
                format!("rule {rule_id:?} contains unsafe path selector {path:?}"),
            );
        }
        for value in selector.labels.iter().chain(&selector.identifiers) {
            self.check(
                !value.is_empty() && value.len() <= 1_024,
                code,
                format!("rule {rule_id:?} has an empty or oversized selector value"),
            );
        }
    }

    fn validate_policy_type(&mut self, semantic_type: &str, rule_id: &str) {
        self.check(
            self.namespaced(semantic_type),
            "policy_type_namespace",
            format!(
                "policy rule {rule_id:?} uses semantic type {semantic_type:?} outside the namespace"
            ),
        );
        self.check(
            self.package
                .mappings
                .nodes
                .iter()
                .any(|mapping| mapping.semantic_type == semantic_type),
            "policy_type_unknown",
            format!(
                "policy rule {rule_id:?} uses semantic type {semantic_type:?} not emitted by a mapping"
            ),
        );
    }

    fn validate_policy_relation(&mut self, relation_type: &str, rule_id: &str) {
        self.check(
            self.namespaced(relation_type),
            "policy_relation_namespace",
            format!(
                "policy rule {rule_id:?} uses relationship {relation_type:?} outside the namespace"
            ),
        );
        self.check(
            self.package
                .mappings
                .edges
                .iter()
                .any(|mapping| mapping.relation_type == relation_type),
            "policy_relation_unknown",
            format!(
                "policy rule {rule_id:?} uses relationship {relation_type:?} not emitted by a mapping"
            ),
        );
    }

    fn namespaced(&self, value: &str) -> bool {
        let prefix = format!("{}::", self.package.manifest.id);
        value
            .strip_prefix(&prefix)
            .is_some_and(|suffix| portable_name(suffix, 128))
    }
}

pub(super) fn validate(package: &AdapterPackage, limits: AdapterLimits) -> ValidationResult {
    Validator {
        package,
        limits,
        checks: 0,
        issues: Vec::new(),
    }
    .validate()
}

pub(super) fn sort_issues(issues: &mut [AdapterConformanceIssue]) {
    issues.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.fixture_id.cmp(&right.fixture_id))
            .then_with(|| left.message.cmp(&right.message))
    });
}

pub(super) fn selector_matches(selector: &RawNodeSelector, node: &qbm_domain::GraphNode) -> bool {
    if !selector.formats.is_empty() && !selector.formats.contains(&node.format) {
        return false;
    }
    if !selector.kinds.is_empty() && !selector.kinds.contains(&node.kind) {
        return false;
    }
    let path = node.evidence.relative_path.as_deref();
    if !selector.path_prefixes.is_empty()
        && !path.is_some_and(|path| {
            selector
                .path_prefixes
                .iter()
                .any(|prefix| path.starts_with(prefix))
        })
    {
        return false;
    }
    if !selector.path_suffixes.is_empty()
        && !path.is_some_and(|path| {
            selector
                .path_suffixes
                .iter()
                .any(|suffix| path.ends_with(suffix))
        })
    {
        return false;
    }
    if !selector.labels.is_empty() && !selector.labels.contains(&node.label) {
        return false;
    }
    if !selector.identifiers.is_empty()
        && !node
            .identifier
            .as_ref()
            .is_some_and(|identifier| selector.identifiers.contains(identifier))
    {
        return false;
    }
    if let Some(required) = selector.identifier_required
        && node.identifier.is_some() != required
    {
        return false;
    }
    if let Some(definition) = selector.definition
        && node.is_definition != definition
    {
        return false;
    }
    true
}

fn portable_name(value: &str, maximum: usize) -> bool {
    value.len() <= maximum
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_short(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.trim() == value
        && value.chars().all(|character| !character.is_control())
}

fn valid_text(value: &str, maximum: usize) -> bool {
    valid_short(value, maximum) && !value.chars().all(char::is_whitespace)
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        && value.len() <= 1_024
}

fn safe_path_fragment(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.split('/').any(|part| matches!(part, "." | ".."))
        && value.len() <= 1_024
}

pub(super) fn records_by_hash(
    records: &[qbm_domain::AdapterRecord],
) -> BTreeMap<&qbm_domain::Sha256Digest, &qbm_domain::AdapterRecord> {
    records
        .iter()
        .map(|record| (&record.package_hash, record))
        .collect()
}
