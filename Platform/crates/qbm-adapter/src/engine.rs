use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
};

use chrono::Utc;
use qbm_canonical::{canonical_json, hash_value};
use qbm_domain::{
    AdapterCandidate, AdapterConformanceIssue, AdapterConformanceReport, AdapterDetectionReport,
    AdapterFixture, AdapterLock, AdapterPackage, AdapterPlan, AdapterRecord, AuditFinding,
    AuditSummary, DetectionRuleOutcome, EvidenceLocation, GraphEdge, GraphNode, MappingValueSource,
    ProjectionCatalog, RawProjectGraph, ResolvedProjection, SemanticAuditReport, SemanticEdge,
    SemanticGraph, SemanticIdentitySource, SemanticNode, SemanticPolicyAssertion, Severity,
    Sha256Digest, SourceFormat, SourceSnapshot,
};
use qbm_store::PlatformStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    AdapterError, AdapterLimits, strict_json,
    validation::{records_by_hash, selector_matches, sort_issues, validate},
};

const DETECTION_SCHEMA: &str = "qbm.adapter-detection/v1";
const PLAN_SCHEMA: &str = "qbm.adapter-plan/v1";
const SEMANTIC_GRAPH_SCHEMA: &str = "qbm.semantic-graph/v1";
const SEMANTIC_REPORT_SCHEMA: &str = "qbm.semantic-audit-report/v1";
const PROJECTION_CATALOG_SCHEMA: &str = "qbm.projection-catalog/v1";
// This is also the frozen behavioral conformance-suite revision. Any change to
// validation, fixture interpretation, or pass/fail semantics must bump it so an
// already installed package is explicitly re-certified instead of silently
// inheriting different trust semantics under the same certificate identity.
const CONFORMANCE_SCHEMA: &str = "qbm.adapter-conformance/v1";

/// Deterministic declarative-adapter interpreter over one local artifact store.
#[derive(Debug, Clone)]
pub struct AdapterEngine {
    store: PlatformStore,
    limits: AdapterLimits,
}

impl AdapterEngine {
    /// Create an engine with conservative built-in resource limits.
    #[must_use]
    pub fn new(store: PlatformStore) -> Self {
        Self::with_limits(store, AdapterLimits::default())
    }

    /// Create an engine with explicit host-controlled resource limits.
    #[must_use]
    pub fn with_limits(store: PlatformStore, limits: AdapterLimits) -> Self {
        Self { store, limits }
    }

    /// Return the immutable limits enforced by this engine.
    #[must_use]
    pub fn limits(&self) -> AdapterLimits {
        self.limits
    }

    /// Decode an adapter package from strict JSON.
    ///
    /// Duplicate keys, trailing content, unknown fields, and oversized input are
    /// rejected. Semantic eligibility is decided separately by
    /// [`Self::conformance_report`].
    pub fn parse_package_json(&self, bytes: &[u8]) -> Result<AdapterPackage, AdapterError> {
        if bytes.len() > self.limits.max_package_bytes {
            return Err(AdapterError::ResourceLimit {
                kind: "package bytes",
                maximum: self.limits.max_package_bytes as u64,
            });
        }
        let value = strict_json::parse(bytes)?;
        Ok(serde_json::from_value(value)?)
    }

    /// Calculate the canonical, key-order-independent package content hash.
    pub fn package_hash(&self, package: &AdapterPackage) -> Result<Sha256Digest, AdapterError> {
        Ok(hash_value(package)?)
    }

    /// Run static and self-contained fixture conformance for one exact package.
    pub fn conformance_report(
        &self,
        package: &AdapterPackage,
    ) -> Result<AdapterConformanceReport, AdapterError> {
        let mut fuel = Fuel::new(self.limits.max_rule_evaluations);
        self.conformance_report_with_fuel(package, &mut fuel)
    }

    fn conformance_report_with_fuel(
        &self,
        package: &AdapterPackage,
        fuel: &mut Fuel,
    ) -> Result<AdapterConformanceReport, AdapterError> {
        let package_hash = self.package_hash(package)?;
        let validation = validate(package, self.limits);
        let mut checks = validation.checks;
        let mut issues = validation.issues;
        if issues.iter().all(|issue| issue.severity < Severity::Error) {
            let mut coverage = FixtureCoverage::default();
            for fixture in &package.fixtures {
                self.run_fixture(
                    package,
                    &package_hash,
                    fixture,
                    &mut checks,
                    &mut issues,
                    &mut coverage,
                    fuel,
                )?;
            }
            verify_fixture_coverage(package, &coverage, &mut checks, &mut issues);
        }
        sort_issues(&mut issues);
        let passed = issues.iter().all(|issue| issue.severity < Severity::Error);
        let id = hash_value(&ConformanceIdentity {
            schema_version: CONFORMANCE_SCHEMA,
            package_hash: &package_hash,
            passed,
            checks,
            issues: &issues,
        })?;
        Ok(AdapterConformanceReport {
            schema_version: CONFORMANCE_SCHEMA.to_owned(),
            id,
            package_hash,
            passed,
            checks,
            issues,
            created_at: Utc::now(),
        })
    }

    /// Score installed adapter versions against one approved raw graph.
    pub fn detect(
        &self,
        graph: &RawProjectGraph,
        records: &[AdapterRecord],
    ) -> Result<AdapterDetectionReport, AdapterError> {
        if records.len() > self.limits.max_registry_packages {
            return Err(AdapterError::ResourceLimit {
                kind: "registry packages",
                maximum: self.limits.max_registry_packages as u64,
            });
        }
        let mut candidates = Vec::with_capacity(records.len());
        let mut unique_hashes = BTreeSet::new();
        let mut fuel = Fuel::new(self.limits.max_rule_evaluations);
        for record in records {
            if !unique_hashes.insert(record.package_hash.clone()) {
                return Err(AdapterError::InvalidRelationship(format!(
                    "registry snapshot repeats package {}",
                    record.package_hash
                )));
            }
            self.validate_record(record, &mut fuel)?;
            candidates.push(detect_package(
                graph,
                &record.package,
                &record.package_hash,
                &mut fuel,
                self.limits,
            )?);
        }
        sort_candidates(&mut candidates);
        if canonical_json(&candidates)?.len() > self.limits.max_total_detection_output_bytes {
            return Err(AdapterError::ResourceLimit {
                kind: "cumulative detection output bytes",
                maximum: self.limits.max_total_detection_output_bytes as u64,
            });
        }
        let id = hash_value(&DetectionIdentity {
            schema_version: DETECTION_SCHEMA,
            graph_id: &graph.id,
            candidates: &candidates,
        })?;
        Ok(AdapterDetectionReport {
            schema_version: DETECTION_SCHEMA.to_owned(),
            id,
            graph_id: graph.id.clone(),
            candidates,
            created_at: Utc::now(),
        })
    }

    /// Lock an approved detection result to selected exact package hashes.
    pub fn create_plan(
        &self,
        detection: &AdapterDetectionReport,
        selected_hashes: &[Sha256Digest],
        records: &[AdapterRecord],
    ) -> Result<AdapterPlan, AdapterError> {
        Self::validate_detection_identity(detection)?;
        if selected_hashes.is_empty() {
            return Err(AdapterError::InvalidRelationship(
                "an adapter plan must select at least one eligible package".to_owned(),
            ));
        }
        if selected_hashes.len() > self.limits.max_selected_adapters {
            return Err(AdapterError::ResourceLimit {
                kind: "selected adapters",
                maximum: self.limits.max_selected_adapters as u64,
            });
        }
        let registry = records_by_hash(records);
        let mut fuel = Fuel::new(self.limits.max_rule_evaluations);
        let mut selected = BTreeSet::new();
        let mut selected_adapter_ids = BTreeSet::new();
        let mut locks = Vec::with_capacity(selected_hashes.len());
        for package_hash in selected_hashes {
            if !selected.insert(package_hash.clone()) {
                return Err(AdapterError::InvalidRelationship(format!(
                    "selected package {package_hash} appears more than once"
                )));
            }
            let candidate = detection
                .candidates
                .iter()
                .find(|candidate| &candidate.package_hash == package_hash)
                .ok_or_else(|| AdapterError::MissingPackage(package_hash.to_string()))?;
            if !candidate.eligible {
                return Err(AdapterError::InvalidRelationship(format!(
                    "adapter {}@{} is not eligible in detection {}",
                    candidate.adapter_id, candidate.adapter_version, detection.id
                )));
            }
            let record = registry
                .get(package_hash)
                .copied()
                .ok_or_else(|| AdapterError::MissingPackage(package_hash.to_string()))?;
            self.validate_record(record, &mut fuel)?;
            if candidate.adapter_id != record.package.manifest.id
                || candidate.adapter_version != record.package.manifest.version
            {
                return Err(AdapterError::InvalidRelationship(format!(
                    "candidate metadata does not match package {package_hash}"
                )));
            }
            if !selected_adapter_ids.insert(candidate.adapter_id.clone()) {
                return Err(AdapterError::InvalidRelationship(format!(
                    "adapter {} can only be selected once per plan",
                    candidate.adapter_id
                )));
            }
            locks.push(lock_from_record(record));
        }
        locks.sort_by(lock_order);
        let configuration_hash = hash_value(&json!({}))?;
        let id = hash_value(&PlanIdentity {
            schema_version: PLAN_SCHEMA,
            detection_report_id: &detection.id,
            raw_graph_id: &detection.graph_id,
            adapters: &locks,
            configuration_hash: &configuration_hash,
        })?;
        Ok(AdapterPlan {
            schema_version: PLAN_SCHEMA.to_owned(),
            id,
            detection_report_id: detection.id.clone(),
            raw_graph_id: detection.graph_id.clone(),
            adapters: locks,
            configuration_hash,
            created_at: Utc::now(),
        })
    }

    /// Project semantic nodes and edges from an approved graph and frozen snapshot.
    pub fn project_with_store(
        &self,
        graph: &RawProjectGraph,
        snapshot: &SourceSnapshot,
        plan: &AdapterPlan,
        records: &[AdapterRecord],
    ) -> Result<SemanticGraph, AdapterError> {
        if graph.id != plan.raw_graph_id {
            return Err(AdapterError::InvalidRelationship(format!(
                "plan {} locks raw graph {}, not {}",
                plan.id, plan.raw_graph_id, graph.id
            )));
        }
        if graph.snapshot_id != snapshot.id || graph.project_id != snapshot.project_id {
            return Err(AdapterError::InvalidRelationship(
                "raw graph and source snapshot bindings do not match".to_owned(),
            ));
        }
        let mut fuel = Fuel::new(self.limits.max_rule_evaluations);
        let selected = self.validate_plan_records(plan, records, &mut fuel)?;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut evidence_cache = EvidenceCache::from_snapshot(snapshot, &mut fuel)?;
        let mut output_budget = OutputBudget::default();
        for (lock, record) in selected {
            let output = project_package(
                graph,
                &record.package,
                &record.package_hash,
                self.limits,
                &mut fuel,
                &mut output_budget,
                |node| self.resolve_evidence_value(node, snapshot, &mut evidence_cache),
            )?;
            append_bounded(
                &mut nodes,
                output.nodes,
                self.limits.max_semantic_nodes,
                "semantic nodes",
            )?;
            append_bounded(
                &mut edges,
                output.edges,
                self.limits.max_semantic_edges,
                "semantic edges",
            )?;
            debug_assert_eq!(&lock.package_hash, &record.package_hash);
        }
        nodes.sort_by(|left, right| left.id.cmp(&right.id));
        edges.sort_by(|left, right| left.id.cmp(&right.id));
        reject_duplicate_ids(nodes.iter().map(|node| &node.id), "semantic node")?;
        reject_duplicate_ids(edges.iter().map(|edge| &edge.id), "semantic edge")?;
        let id = hash_value(&SemanticGraphIdentity {
            schema_version: SEMANTIC_GRAPH_SCHEMA,
            raw_graph_id: &graph.id,
            adapter_plan_id: &plan.id,
            adapters: &plan.adapters,
            nodes: &nodes,
            edges: &edges,
        })?;
        Ok(SemanticGraph {
            schema_version: SEMANTIC_GRAPH_SCHEMA.to_owned(),
            id,
            raw_graph_id: graph.id.clone(),
            adapter_plan_id: plan.id.clone(),
            adapters: plan.adapters.clone(),
            nodes,
            edges,
            created_at: Utc::now(),
        })
    }

    /// Evaluate selected packages' semantic policies over one approved graph.
    pub fn audit(
        &self,
        graph: &SemanticGraph,
        plan: &AdapterPlan,
        records: &[AdapterRecord],
    ) -> Result<SemanticAuditReport, AdapterError> {
        Self::validate_semantic_chain(graph, plan)?;
        let mut fuel = Fuel::new(self.limits.max_rule_evaluations);
        let selected = self.validate_plan_records(plan, records, &mut fuel)?;
        let mut findings = Vec::new();
        let mut audit_budget = AuditBudget::default();
        for (_, record) in selected {
            let package_findings = audit_package(
                graph,
                &record.package,
                &record.package_hash,
                self.limits.max_findings.saturating_sub(findings.len()),
                &mut fuel,
                &mut audit_budget,
                self.limits,
            )?;
            append_bounded(
                &mut findings,
                package_findings,
                self.limits.max_findings,
                "semantic findings",
            )?;
        }
        sort_findings(&mut findings);
        let summary = summarize(&findings);
        let id = hash_value(&SemanticReportIdentity {
            schema_version: SEMANTIC_REPORT_SCHEMA,
            semantic_graph_id: &graph.id,
            adapter_plan_id: &plan.id,
            findings: &findings,
            summary: &summary,
        })?;
        Ok(SemanticAuditReport {
            schema_version: SEMANTIC_REPORT_SCHEMA.to_owned(),
            id,
            semantic_graph_id: graph.id.clone(),
            adapter_plan_id: plan.id.clone(),
            findings,
            summary,
            created_at: Utc::now(),
        })
    }

    /// Resolve selected packages' named projections against an audited semantic graph.
    pub fn resolve_catalog(
        &self,
        graph: &SemanticGraph,
        report: &SemanticAuditReport,
        plan: &AdapterPlan,
        records: &[AdapterRecord],
    ) -> Result<ProjectionCatalog, AdapterError> {
        Self::validate_semantic_chain(graph, plan)?;
        if report.semantic_graph_id != graph.id || report.adapter_plan_id != plan.id {
            return Err(AdapterError::InvalidRelationship(
                "semantic report does not audit the supplied graph and plan".to_owned(),
            ));
        }
        Self::validate_semantic_report_identity(report)?;
        let mut fuel = Fuel::new(self.limits.max_rule_evaluations);
        let selected = self.validate_plan_records(plan, records, &mut fuel)?;
        let mut projections = Vec::new();
        let mut catalog_budget = CatalogBudget::default();
        for (_, record) in selected {
            projections.extend(resolve_package_projections(
                graph,
                &record.package,
                &record.package_hash,
                &mut fuel,
                &mut catalog_budget,
                self.limits,
            )?);
        }
        projections.sort_by(|left, right| {
            left.adapter_id
                .cmp(&right.adapter_id)
                .then_with(|| left.package_hash.cmp(&right.package_hash))
                .then_with(|| left.projection_id.cmp(&right.projection_id))
        });
        let id = hash_value(&CatalogIdentity {
            schema_version: PROJECTION_CATALOG_SCHEMA,
            semantic_graph_id: &graph.id,
            semantic_report_id: &report.id,
            adapter_plan_id: &plan.id,
            projections: &projections,
        })?;
        Ok(ProjectionCatalog {
            schema_version: PROJECTION_CATALOG_SCHEMA.to_owned(),
            id,
            semantic_graph_id: graph.id.clone(),
            semantic_report_id: report.id.clone(),
            adapter_plan_id: plan.id.clone(),
            projections,
            created_at: Utc::now(),
        })
    }

    fn validate_record(&self, record: &AdapterRecord, fuel: &mut Fuel) -> Result<(), AdapterError> {
        let maximum_bytes = u64::try_from(self.limits.max_package_bytes).unwrap_or(u64::MAX);
        let source_bytes = self
            .store
            .read_artifact_bounded(&record.source_hash, maximum_bytes)?;
        fuel.consume(u64::try_from(source_bytes.len()).unwrap_or(u64::MAX))?;
        let source_package = self.parse_package_json(&source_bytes)?;
        let source_package_hash = self.package_hash(&source_package)?;
        if source_package != record.package || source_package_hash != record.package_hash {
            return Err(AdapterError::InvalidRelationship(format!(
                "source artifact {} does not decode to registered package {}",
                record.source_hash, record.package_hash
            )));
        }
        let package_hash = self.package_hash(&record.package)?;
        if package_hash != record.package_hash {
            return Err(AdapterError::InvalidRelationship(format!(
                "registry record says package {}, canonical content is {package_hash}",
                record.package_hash
            )));
        }
        let report = self.conformance_report_with_fuel(&record.package, fuel)?;
        if !report.passed || report.id != record.conformance_report_id {
            return Err(AdapterError::NonConformant(record.package_hash.to_string()));
        }
        Ok(())
    }

    fn validate_plan_records<'a>(
        &self,
        plan: &'a AdapterPlan,
        records: &'a [AdapterRecord],
        fuel: &mut Fuel,
    ) -> Result<Vec<(&'a AdapterLock, &'a AdapterRecord)>, AdapterError> {
        Self::validate_plan_identity(plan)?;
        let registry = records_by_hash(records);
        let mut output = Vec::with_capacity(plan.adapters.len());
        let mut seen_adapters = BTreeSet::new();
        for lock in &plan.adapters {
            if !seen_adapters.insert(lock.adapter_id.clone()) {
                return Err(AdapterError::InvalidRelationship(format!(
                    "plan selects adapter {} more than once",
                    lock.adapter_id
                )));
            }
            let record = registry
                .get(&lock.package_hash)
                .copied()
                .ok_or_else(|| AdapterError::MissingPackage(lock.package_hash.to_string()))?;
            self.validate_record(record, fuel)?;
            let expected = lock_from_record(record);
            if expected != *lock {
                return Err(AdapterError::InvalidRelationship(format!(
                    "plan lock for {} does not match installed package {}",
                    lock.adapter_id, lock.package_hash
                )));
            }
            output.push((lock, record));
        }
        Ok(output)
    }

    fn validate_detection_identity(detection: &AdapterDetectionReport) -> Result<(), AdapterError> {
        let id = hash_value(&DetectionIdentity {
            schema_version: DETECTION_SCHEMA,
            graph_id: &detection.graph_id,
            candidates: &detection.candidates,
        })?;
        if detection.schema_version != DETECTION_SCHEMA || detection.id != id {
            return Err(AdapterError::InvalidRelationship(
                "adapter detection identity or schema is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_plan_identity(plan: &AdapterPlan) -> Result<(), AdapterError> {
        let id = hash_value(&PlanIdentity {
            schema_version: PLAN_SCHEMA,
            detection_report_id: &plan.detection_report_id,
            raw_graph_id: &plan.raw_graph_id,
            adapters: &plan.adapters,
            configuration_hash: &plan.configuration_hash,
        })?;
        if plan.schema_version != PLAN_SCHEMA || plan.id != id {
            return Err(AdapterError::InvalidRelationship(
                "adapter plan identity or schema is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_semantic_chain(
        graph: &SemanticGraph,
        plan: &AdapterPlan,
    ) -> Result<(), AdapterError> {
        Self::validate_plan_identity(plan)?;
        if graph.raw_graph_id != plan.raw_graph_id
            || graph.adapter_plan_id != plan.id
            || graph.adapters != plan.adapters
        {
            return Err(AdapterError::InvalidRelationship(
                "semantic graph does not match the supplied adapter plan".to_owned(),
            ));
        }
        let id = hash_value(&SemanticGraphIdentity {
            schema_version: SEMANTIC_GRAPH_SCHEMA,
            raw_graph_id: &graph.raw_graph_id,
            adapter_plan_id: &graph.adapter_plan_id,
            adapters: &graph.adapters,
            nodes: &graph.nodes,
            edges: &graph.edges,
        })?;
        if graph.schema_version != SEMANTIC_GRAPH_SCHEMA || graph.id != id {
            return Err(AdapterError::InvalidRelationship(
                "semantic graph identity or schema is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_semantic_report_identity(report: &SemanticAuditReport) -> Result<(), AdapterError> {
        let id = hash_value(&SemanticReportIdentity {
            schema_version: SEMANTIC_REPORT_SCHEMA,
            semantic_graph_id: &report.semantic_graph_id,
            adapter_plan_id: &report.adapter_plan_id,
            findings: &report.findings,
            summary: &report.summary,
        })?;
        if report.schema_version != SEMANTIC_REPORT_SCHEMA || report.id != id {
            return Err(AdapterError::InvalidRelationship(
                "semantic report identity or schema is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    fn resolve_evidence_value(
        &self,
        node: &GraphNode,
        snapshot: &SourceSnapshot,
        cache: &mut EvidenceCache,
    ) -> Result<Option<Value>, AdapterError> {
        let Some(artifact_hash) = &node.evidence.artifact_sha256 else {
            return Ok(None);
        };
        let Some(relative_path) = &node.evidence.relative_path else {
            return Err(AdapterError::Evidence(format!(
                "node {} has an artifact hash without a relative path",
                node.id
            )));
        };
        let Some(binding_size) = cache
            .bindings
            .get(artifact_hash)
            .and_then(|paths| paths.get(relative_path))
        else {
            return Err(AdapterError::Evidence(format!(
                "node {} points outside snapshot {}",
                node.id, snapshot.id
            )));
        };
        if *binding_size > self.limits.max_evidence_bytes {
            return Err(AdapterError::ResourceLimit {
                kind: "evidence artifact bytes",
                maximum: self.limits.max_evidence_bytes,
            });
        }
        let key = (artifact_hash.clone(), node.format);
        if !cache.documents.contains_key(&key) {
            let bytes = self
                .store
                .read_artifact_bounded(artifact_hash, self.limits.max_evidence_bytes)?;
            cache.total_bytes = cache.total_bytes.saturating_add(bytes.len() as u64);
            if cache.total_bytes > self.limits.max_total_evidence_bytes {
                return Err(AdapterError::ResourceLimit {
                    kind: "cumulative evidence bytes",
                    maximum: self.limits.max_total_evidence_bytes,
                });
            }
            let source = std::str::from_utf8(&bytes).map_err(|error| {
                AdapterError::Evidence(format!("artifact {artifact_hash} is not UTF-8: {error}"))
            })?;
            let document = parse_document(source, node.format, self.limits)?;
            cache.documents.insert(key.clone(), document);
        }
        Ok(cache
            .documents
            .get(&key)
            .and_then(|document| document_value(document, node.evidence.pointer.as_deref())))
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn run_fixture(
        &self,
        package: &AdapterPackage,
        package_hash: &Sha256Digest,
        fixture: &AdapterFixture,
        checks: &mut u64,
        issues: &mut Vec<AdapterConformanceIssue>,
        coverage: &mut FixtureCoverage,
        fuel: &mut Fuel,
    ) -> Result<(), AdapterError> {
        let built = build_fixture_graph(package_hash, fixture)?;
        fuel.consume((fixture.nodes.len().saturating_add(fixture.edges.len())) as u64)?;
        let candidate = detect_package(&built.graph, package, package_hash, fuel, self.limits)?;
        coverage.detection_rules.extend(
            candidate
                .rules
                .iter()
                .filter(|rule| rule.matched)
                .map(|rule| rule.rule_id.clone()),
        );
        fixture_check(
            checks,
            issues,
            &fixture.id,
            candidate.eligible == fixture.expect.detection_eligible,
            "fixture_detection_eligibility",
            format!(
                "expected detection eligibility {}, got {}",
                fixture.expect.detection_eligible, candidate.eligible
            ),
        );
        fixture_check(
            checks,
            issues,
            &fixture.id,
            candidate.confidence_bps >= fixture.expect.minimum_confidence_bps,
            "fixture_detection_confidence",
            format!(
                "expected at least {} bps, got {} bps",
                fixture.expect.minimum_confidence_bps, candidate.confidence_bps
            ),
        );
        let projected = project_package(
            &built.graph,
            package,
            package_hash,
            self.limits,
            fuel,
            &mut OutputBudget::default(),
            |node| Ok(built.values.get(&node.id).cloned().flatten()),
        )?;
        let actual_counts = count_types(&projected.nodes);
        coverage.node_mappings.extend(
            projected
                .nodes
                .iter()
                .map(|node| node.mapping_rule_id.clone()),
        );
        fixture_check(
            checks,
            issues,
            &fixture.id,
            actual_counts == fixture.expect.semantic_type_counts,
            "fixture_semantic_counts",
            format!(
                "expected semantic counts {:?}, got {actual_counts:?}",
                fixture.expect.semantic_type_counts
            ),
        );
        let actual_relation_counts = count_relations(&projected.edges);
        coverage.edge_mappings.extend(
            projected
                .edges
                .iter()
                .map(|edge| edge.mapping_rule_id.clone()),
        );
        fixture_check(
            checks,
            issues,
            &fixture.id,
            actual_relation_counts == fixture.expect.semantic_relation_counts,
            "fixture_semantic_relation_counts",
            format!(
                "expected semantic relationship counts {:?}, got {actual_relation_counts:?}",
                fixture.expect.semantic_relation_counts
            ),
        );
        let semantic_graph = fixture_semantic_graph(package, package_hash, projected)?;
        let findings = audit_package(
            &semantic_graph,
            package,
            package_hash,
            self.limits.max_findings,
            fuel,
            &mut AuditBudget::default(),
            self.limits,
        )?;
        let codes: BTreeSet<_> = findings
            .iter()
            .map(|finding| finding.code.clone())
            .collect();
        coverage.finding_codes.extend(codes.iter().cloned());
        fixture_check(
            checks,
            issues,
            &fixture.id,
            fixture.expect.finding_codes == codes,
            "fixture_findings",
            format!(
                "expected finding codes {:?}, got {codes:?}",
                fixture.expect.finding_codes
            ),
        );
        let projections = resolve_package_projections(
            &semantic_graph,
            package,
            package_hash,
            fuel,
            &mut CatalogBudget::default(),
            self.limits,
        )?;
        let availability: BTreeMap<_, _> = projections
            .iter()
            .map(|projection| (projection.projection_id.clone(), projection.available))
            .collect();
        coverage.projection_ids.extend(
            projections
                .iter()
                .map(|projection| projection.projection_id.clone()),
        );
        fixture_check(
            checks,
            issues,
            &fixture.id,
            availability == fixture.expect.projection_availability,
            "fixture_projection_availability",
            format!(
                "expected projection availability {:?}, got {availability:?}",
                fixture.expect.projection_availability
            ),
        );
        Ok(())
    }
}

#[derive(Serialize)]
struct ConformanceIdentity<'a> {
    schema_version: &'static str,
    package_hash: &'a Sha256Digest,
    passed: bool,
    checks: u64,
    issues: &'a [AdapterConformanceIssue],
}

#[derive(Serialize)]
struct DetectionIdentity<'a> {
    schema_version: &'static str,
    graph_id: &'a Sha256Digest,
    candidates: &'a [AdapterCandidate],
}

#[derive(Serialize)]
struct PlanIdentity<'a> {
    schema_version: &'static str,
    detection_report_id: &'a Sha256Digest,
    raw_graph_id: &'a Sha256Digest,
    adapters: &'a [AdapterLock],
    configuration_hash: &'a Sha256Digest,
}

#[derive(Serialize)]
struct SemanticGraphIdentity<'a> {
    schema_version: &'static str,
    raw_graph_id: &'a Sha256Digest,
    adapter_plan_id: &'a Sha256Digest,
    adapters: &'a [AdapterLock],
    nodes: &'a [SemanticNode],
    edges: &'a [SemanticEdge],
}

#[derive(Serialize)]
struct SemanticReportIdentity<'a> {
    schema_version: &'static str,
    semantic_graph_id: &'a Sha256Digest,
    adapter_plan_id: &'a Sha256Digest,
    findings: &'a [AuditFinding],
    summary: &'a AuditSummary,
}

#[derive(Serialize)]
struct CatalogIdentity<'a> {
    schema_version: &'static str,
    semantic_graph_id: &'a Sha256Digest,
    semantic_report_id: &'a Sha256Digest,
    adapter_plan_id: &'a Sha256Digest,
    projections: &'a [ResolvedProjection],
}

#[derive(Serialize)]
struct SemanticNodeIdentity<'a> {
    schema_version: &'static str,
    adapter_id: &'a qbm_domain::AdapterId,
    package_hash: &'a Sha256Digest,
    mapping_rule_id: &'a str,
    semantic_type: &'a str,
    external_id: &'a str,
    attributes: &'a BTreeMap<String, Value>,
    source_node: &'a Sha256Digest,
}

#[derive(Serialize)]
struct SemanticEdgeIdentity<'a> {
    schema_version: &'static str,
    adapter_id: &'a qbm_domain::AdapterId,
    package_hash: &'a Sha256Digest,
    mapping_rule_id: &'a str,
    relation_type: &'a str,
    from: &'a Sha256Digest,
    to: &'a Sha256Digest,
    source_edge: &'a Sha256Digest,
}

#[derive(Serialize)]
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

#[derive(Serialize)]
struct FixtureRawIdentity<'a> {
    schema_version: &'static str,
    package_hash: &'a Sha256Digest,
    fixture_id: &'a str,
    nodes: &'a [GraphNode],
    edges: &'a [GraphEdge],
}

#[derive(Serialize)]
struct FixtureElementIdentity<'a> {
    schema_version: &'static str,
    package_hash: &'a Sha256Digest,
    fixture_id: &'a str,
    key: &'a str,
}

struct ProjectedPackage {
    nodes: Vec<SemanticNode>,
    edges: Vec<SemanticEdge>,
}

struct BuiltFixture {
    graph: RawProjectGraph,
    values: BTreeMap<Sha256Digest, Option<Value>>,
}

#[derive(Debug, Default)]
struct FixtureCoverage {
    detection_rules: BTreeSet<String>,
    node_mappings: BTreeSet<String>,
    edge_mappings: BTreeSet<String>,
    finding_codes: BTreeSet<String>,
    projection_ids: BTreeSet<String>,
}

#[derive(Debug)]
struct Fuel {
    remaining: u64,
    maximum: u64,
}

impl Fuel {
    fn new(maximum: u64) -> Self {
        Self {
            remaining: maximum,
            maximum,
        }
    }

    fn consume(&mut self, amount: u64) -> Result<(), AdapterError> {
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .ok_or(AdapterError::ResourceLimit {
                kind: "rule evaluations",
                maximum: self.maximum,
            })?;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct EvidenceCache {
    bindings: BTreeMap<Sha256Digest, BTreeMap<String, u64>>,
    documents: BTreeMap<(Sha256Digest, SourceFormat), ParsedDocument>,
    total_bytes: u64,
}

impl EvidenceCache {
    fn from_snapshot(snapshot: &SourceSnapshot, fuel: &mut Fuel) -> Result<Self, AdapterError> {
        let mut cache = Self::default();
        let mut seen_paths = BTreeSet::new();
        for artifact in &snapshot.artifacts {
            fuel.consume(1)?;
            if !seen_paths.insert(artifact.relative_path.clone()) {
                return Err(AdapterError::InvalidRelationship(format!(
                    "snapshot {} repeats relative path {:?}",
                    snapshot.id, artifact.relative_path
                )));
            }
            let previous = cache
                .bindings
                .entry(artifact.sha256.clone())
                .or_default()
                .insert(artifact.relative_path.clone(), artifact.size_bytes);
            if previous.is_some() {
                return Err(AdapterError::InvalidRelationship(format!(
                    "snapshot {} repeats artifact/path binding {} at {:?}",
                    snapshot.id, artifact.sha256, artifact.relative_path
                )));
            }
        }
        Ok(cache)
    }
}

#[derive(Debug, Default)]
struct OutputBudget {
    attribute_bytes: usize,
    semantic_bytes: usize,
}

#[derive(Debug, Default)]
struct AuditBudget {
    canonical_bytes: usize,
}

#[derive(Debug, Default)]
struct CatalogBudget {
    members: usize,
    canonical_bytes: usize,
}

#[derive(Debug)]
enum ParsedDocument {
    Single(Value),
    Yaml(Vec<Value>),
    Unsupported,
}

fn fixture_check(
    checks: &mut u64,
    issues: &mut Vec<AdapterConformanceIssue>,
    fixture_id: &str,
    condition: bool,
    code: &str,
    message: String,
) {
    *checks = checks.saturating_add(1);
    if !condition {
        issues.push(AdapterConformanceIssue {
            code: code.to_owned(),
            severity: Severity::Error,
            message,
            fixture_id: Some(fixture_id.to_owned()),
        });
    }
}

fn coverage_check(
    checks: &mut u64,
    issues: &mut Vec<AdapterConformanceIssue>,
    condition: bool,
    code: &str,
    message: String,
) {
    *checks = checks.saturating_add(1);
    if !condition {
        issues.push(AdapterConformanceIssue {
            code: code.to_owned(),
            severity: Severity::Error,
            message,
            fixture_id: None,
        });
    }
}

fn verify_fixture_coverage(
    package: &AdapterPackage,
    coverage: &FixtureCoverage,
    checks: &mut u64,
    issues: &mut Vec<AdapterConformanceIssue>,
) {
    for rule in &package.detection {
        coverage_check(
            checks,
            issues,
            coverage.detection_rules.contains(&rule.id),
            "fixture_detection_rule_uncovered",
            format!(
                "detection rule {:?} matches no conformance fixture",
                rule.id
            ),
        );
    }
    for rule in &package.mappings.nodes {
        coverage_check(
            checks,
            issues,
            coverage.node_mappings.contains(&rule.id),
            "fixture_node_mapping_uncovered",
            format!("node mapping {:?} emits no fixture output", rule.id),
        );
    }
    for rule in &package.mappings.edges {
        coverage_check(
            checks,
            issues,
            coverage.edge_mappings.contains(&rule.id),
            "fixture_edge_mapping_uncovered",
            format!("edge mapping {:?} emits no fixture output", rule.id),
        );
    }
    for rule in &package.policies.rules {
        let code = format!("{}::{}", package.manifest.id, rule.id);
        coverage_check(
            checks,
            issues,
            coverage.finding_codes.contains(&code),
            "fixture_policy_rule_uncovered",
            format!("policy rule {:?} produces no fixture finding", rule.id),
        );
    }
    for projection in &package.projections {
        coverage_check(
            checks,
            issues,
            coverage.projection_ids.contains(&projection.id),
            "fixture_projection_uncovered",
            format!(
                "projection {:?} is absent from fixture expectations",
                projection.id
            ),
        );
    }
}

#[allow(clippy::too_many_lines)]
fn build_fixture_graph(
    package_hash: &Sha256Digest,
    fixture: &AdapterFixture,
) -> Result<BuiltFixture, AdapterError> {
    let mut nodes = Vec::with_capacity(fixture.nodes.len());
    let mut values = BTreeMap::new();
    let mut node_ids = BTreeMap::new();
    for fixture_node in &fixture.nodes {
        let id = hash_value(&FixtureElementIdentity {
            schema_version: "qbm.adapter-fixture-node/v1",
            package_hash,
            fixture_id: &fixture.id,
            key: &fixture_node.key,
        })?;
        let artifact_sha256 = hash_value(&FixtureElementIdentity {
            schema_version: "qbm.adapter-fixture-evidence/v1",
            package_hash,
            fixture_id: &fixture.id,
            key: &fixture_node.key,
        })?;
        let evidence = EvidenceLocation {
            artifact_sha256: Some(artifact_sha256),
            relative_path: Some(fixture_node.relative_path.clone()),
            pointer: fixture_node.pointer.clone(),
            line: None,
            column: None,
        };
        nodes.push(GraphNode {
            id: id.clone(),
            kind: fixture_node.kind,
            label: fixture_node.label.clone(),
            format: fixture_node.format,
            identifier: fixture_node.identifier.clone(),
            is_definition: fixture_node.is_definition,
            properties: fixture_node.properties.clone(),
            evidence,
        });
        node_ids.insert(fixture_node.key.clone(), id.clone());
        values.insert(id, fixture_node.evidence_value.clone());
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));

    let nodes_by_id: BTreeMap<_, _> = nodes.iter().map(|node| (&node.id, node)).collect();
    let mut edges = Vec::with_capacity(fixture.edges.len());
    for (index, fixture_edge) in fixture.edges.iter().enumerate() {
        let from = node_ids.get(&fixture_edge.from).cloned().ok_or_else(|| {
            AdapterError::InvalidRelationship(format!(
                "fixture {} edge source {:?} does not exist",
                fixture.id, fixture_edge.from
            ))
        })?;
        let to = fixture_edge
            .to
            .as_ref()
            .map(|key| {
                node_ids.get(key).cloned().ok_or_else(|| {
                    AdapterError::InvalidRelationship(format!(
                        "fixture {} edge target {key:?} does not exist",
                        fixture.id
                    ))
                })
            })
            .transpose()?;
        let edge_key = format!("edge-{index}");
        let id = hash_value(&FixtureElementIdentity {
            schema_version: "qbm.adapter-fixture-edge/v1",
            package_hash,
            fixture_id: &fixture.id,
            key: &edge_key,
        })?;
        let evidence = nodes_by_id
            .get(&from)
            .map(|node| node.evidence.clone())
            .ok_or_else(|| {
                AdapterError::InvalidRelationship(format!(
                    "fixture {} cannot resolve edge evidence",
                    fixture.id
                ))
            })?;
        edges.push(GraphEdge {
            id,
            from,
            to,
            kind: fixture_edge.kind,
            reference: fixture_edge.reference.clone(),
            evidence,
        });
    }
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    let id = hash_value(&FixtureRawIdentity {
        schema_version: "qbm.adapter-fixture-raw-graph/v1",
        package_hash,
        fixture_id: &fixture.id,
        nodes: &nodes,
        edges: &edges,
    })?;
    let snapshot_id = hash_value(&json!({
        "schema_version": "qbm.adapter-fixture-snapshot/v1",
        "package_hash": package_hash,
        "fixture_id": fixture.id,
    }))?;
    let policy_hash = hash_value(&json!({
        "schema_version": "qbm.adapter-fixture-graph-policy/v1",
        "package_hash": package_hash,
        "fixture_id": fixture.id,
    }))?;
    let project_id = qbm_domain::ProjectId::new("qbm-fixture")
        .map_err(|error| AdapterError::InvalidRelationship(error.to_string()))?;
    Ok(BuiltFixture {
        graph: RawProjectGraph {
            schema_version: qbm_domain::DOMAIN_SCHEMA_VERSION.to_owned(),
            id,
            snapshot_id,
            project_id,
            policy_hash,
            nodes,
            edges,
            diagnostics: Vec::new(),
            created_at: Utc::now(),
        },
        values,
    })
}

fn count_types(nodes: &[SemanticNode]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for node in nodes {
        let count = counts.entry(node.semantic_type.clone()).or_insert(0_u64);
        *count = count.saturating_add(1);
    }
    counts
}

fn count_relations(edges: &[SemanticEdge]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for edge in edges {
        let count = counts.entry(edge.relation_type.clone()).or_insert(0_u64);
        *count = count.saturating_add(1);
    }
    counts
}

fn fixture_semantic_graph(
    package: &AdapterPackage,
    package_hash: &Sha256Digest,
    projected: ProjectedPackage,
) -> Result<SemanticGraph, AdapterError> {
    let raw_graph_id = hash_value(&json!({
        "schema_version": "qbm.adapter-fixture-semantic-source/v1",
        "package_hash": package_hash,
    }))?;
    let adapter_plan_id = hash_value(&json!({
        "schema_version": "qbm.adapter-fixture-plan/v1",
        "package_hash": package_hash,
    }))?;
    let conformance_report_id = hash_value(&json!({
        "schema_version": "qbm.adapter-fixture-conformance/v1",
        "package_hash": package_hash,
    }))?;
    let adapters = vec![AdapterLock {
        adapter_id: package.manifest.id.clone(),
        adapter_version: package.manifest.version.clone(),
        package_hash: package_hash.clone(),
        conformance_report_id,
        capabilities: package.manifest.capabilities.clone(),
    }];
    let id = hash_value(&SemanticGraphIdentity {
        schema_version: SEMANTIC_GRAPH_SCHEMA,
        raw_graph_id: &raw_graph_id,
        adapter_plan_id: &adapter_plan_id,
        adapters: &adapters,
        nodes: &projected.nodes,
        edges: &projected.edges,
    })?;
    Ok(SemanticGraph {
        schema_version: SEMANTIC_GRAPH_SCHEMA.to_owned(),
        id,
        raw_graph_id,
        adapter_plan_id,
        adapters,
        nodes: projected.nodes,
        edges: projected.edges,
        created_at: Utc::now(),
    })
}

fn detect_package(
    graph: &RawProjectGraph,
    package: &AdapterPackage,
    package_hash: &Sha256Digest,
    fuel: &mut Fuel,
    limits: AdapterLimits,
) -> Result<AdapterCandidate, AdapterError> {
    let mut rules = Vec::with_capacity(package.detection.len());
    let mut raw_nodes: Vec<_> = graph.nodes.iter().collect();
    raw_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    for rule in &package.detection {
        let mut match_count = 0_u64;
        let mut matching_node_samples = Vec::new();
        for node in &raw_nodes {
            fuel.consume(1)?;
            if selector_matches(&rule.selector, node) {
                match_count = match_count.saturating_add(1);
                if matching_node_samples.len() < limits.max_detection_evidence_samples {
                    matching_node_samples.push(node.id.clone());
                }
            }
        }
        matching_node_samples.sort();
        rules.push(DetectionRuleOutcome {
            rule_id: rule.id.clone(),
            match_count,
            matched: match_count >= rule.minimum_matches,
            required: rule.required,
            weight: rule.weight,
            matching_node_samples,
        });
    }
    rules.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
    let matched_weight = rules
        .iter()
        .filter(|rule| rule.matched)
        .map(|rule| u64::from(rule.weight))
        .sum();
    let maximum_weight = rules.iter().map(|rule| u64::from(rule.weight)).sum();
    let eligible = rules.iter().all(|rule| !rule.required || rule.matched);
    let confidence = if maximum_weight == 0 {
        0
    } else {
        (u128::from(matched_weight) * 10_000) / u128::from(maximum_weight)
    };
    Ok(AdapterCandidate {
        adapter_id: package.manifest.id.clone(),
        adapter_version: package.manifest.version.clone(),
        package_hash: package_hash.clone(),
        eligible,
        confidence_bps: u16::try_from(confidence).unwrap_or(10_000),
        matched_weight,
        maximum_weight,
        rules,
    })
}

fn sort_candidates(candidates: &mut [AdapterCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .eligible
            .cmp(&left.eligible)
            .then_with(|| right.confidence_bps.cmp(&left.confidence_bps))
            .then_with(|| left.adapter_id.cmp(&right.adapter_id))
            .then_with(|| left.adapter_version.cmp(&right.adapter_version))
            .then_with(|| left.package_hash.cmp(&right.package_hash))
    });
}

fn lock_from_record(record: &AdapterRecord) -> AdapterLock {
    AdapterLock {
        adapter_id: record.package.manifest.id.clone(),
        adapter_version: record.package.manifest.version.clone(),
        package_hash: record.package_hash.clone(),
        conformance_report_id: record.conformance_report_id.clone(),
        capabilities: record.package.manifest.capabilities.clone(),
    }
}

fn lock_order(left: &AdapterLock, right: &AdapterLock) -> std::cmp::Ordering {
    left.adapter_id
        .cmp(&right.adapter_id)
        .then_with(|| left.adapter_version.cmp(&right.adapter_version))
        .then_with(|| left.package_hash.cmp(&right.package_hash))
}

#[allow(clippy::too_many_lines)]
fn project_package<F>(
    graph: &RawProjectGraph,
    package: &AdapterPackage,
    package_hash: &Sha256Digest,
    limits: AdapterLimits,
    fuel: &mut Fuel,
    output_budget: &mut OutputBudget,
    mut evidence_value: F,
) -> Result<ProjectedPackage, AdapterError>
where
    F: FnMut(&GraphNode) -> Result<Option<Value>, AdapterError>,
{
    let mut raw_nodes: Vec<_> = graph.nodes.iter().collect();
    raw_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let mut node_rules: Vec<_> = package.mappings.nodes.iter().collect();
    node_rules.sort_by(|left, right| left.id.cmp(&right.id));
    let mut nodes = Vec::new();
    let mut by_raw: BTreeMap<Sha256Digest, Vec<usize>> = BTreeMap::new();
    for rule in node_rules {
        for raw in &raw_nodes {
            fuel.consume(1)?;
            if !selector_matches(&rule.selector, raw) {
                continue;
            }
            if nodes.len() >= limits.max_semantic_nodes {
                return Err(AdapterError::ResourceLimit {
                    kind: "semantic nodes",
                    maximum: limits.max_semantic_nodes as u64,
                });
            }
            let mut attributes = BTreeMap::new();
            let mut pending_attribute_bytes = 0_usize;
            let mut missing_required = false;
            for attribute in &rule.attributes {
                let value = mapping_value(raw, &attribute.value, &mut evidence_value)?;
                match value {
                    Some(value) => {
                        let value_bytes =
                            qbm_canonical::canonical_json(&(attribute.target.as_str(), &value))?
                                .len();
                        pending_attribute_bytes =
                            pending_attribute_bytes.saturating_add(value_bytes);
                        if pending_attribute_bytes > limits.max_attribute_bytes {
                            return Err(AdapterError::ResourceLimit {
                                kind: "semantic attribute bytes",
                                maximum: limits.max_attribute_bytes as u64,
                            });
                        }
                        attributes.insert(attribute.target.clone(), value);
                    }
                    None if attribute.required => {
                        missing_required = true;
                        break;
                    }
                    None => {}
                }
            }
            if missing_required {
                continue;
            }
            let attribute_bytes = qbm_canonical::canonical_json(&attributes)?.len();
            if attribute_bytes > limits.max_attribute_bytes {
                return Err(AdapterError::ResourceLimit {
                    kind: "semantic attribute bytes",
                    maximum: limits.max_attribute_bytes as u64,
                });
            }
            output_budget.attribute_bytes = output_budget
                .attribute_bytes
                .saturating_add(attribute_bytes);
            if output_budget.attribute_bytes > limits.max_total_attribute_bytes {
                return Err(AdapterError::ResourceLimit {
                    kind: "cumulative semantic attribute bytes",
                    maximum: limits.max_total_attribute_bytes as u64,
                });
            }
            let external_id = semantic_external_id(raw, rule.identity_source);
            let id = hash_value(&SemanticNodeIdentity {
                schema_version: "qbm.semantic-node-identity/v1",
                adapter_id: &package.manifest.id,
                package_hash,
                mapping_rule_id: &rule.id,
                semantic_type: &rule.semantic_type,
                external_id: &external_id,
                attributes: &attributes,
                source_node: &raw.id,
            })?;
            let index = nodes.len();
            let node = SemanticNode {
                id,
                adapter_id: package.manifest.id.clone(),
                package_hash: package_hash.clone(),
                mapping_rule_id: rule.id.clone(),
                semantic_type: rule.semantic_type.clone(),
                external_id,
                label: raw.label.clone(),
                attributes,
                source_node: raw.id.clone(),
                evidence: vec![raw.evidence.clone()],
            };
            charge_semantic_output(output_budget, &node, limits)?;
            nodes.push(node);
            by_raw.entry(raw.id.clone()).or_default().push(index);
        }
    }

    let mut edge_rules: Vec<_> = package.mappings.edges.iter().collect();
    edge_rules.sort_by(|left, right| left.id.cmp(&right.id));
    let mut raw_edges: Vec<_> = graph.edges.iter().collect();
    raw_edges.sort_by(|left, right| left.id.cmp(&right.id));
    let mut edges = Vec::new();
    for rule in edge_rules {
        for raw_edge in &raw_edges {
            fuel.consume(1)?;
            if !rule.raw_kinds.is_empty() && !rule.raw_kinds.contains(&raw_edge.kind) {
                continue;
            }
            let Some(target_raw) = &raw_edge.to else {
                continue;
            };
            let Some(sources) = by_raw.get(&raw_edge.from) else {
                continue;
            };
            let Some(targets) = by_raw.get(target_raw) else {
                continue;
            };
            for source_index in sources {
                fuel.consume(1)?;
                let source = &nodes[*source_index];
                if rule
                    .from_semantic_type
                    .as_ref()
                    .is_some_and(|required| source.semantic_type != *required)
                {
                    continue;
                }
                for target_index in targets {
                    fuel.consume(1)?;
                    let target = &nodes[*target_index];
                    if rule
                        .to_semantic_type
                        .as_ref()
                        .is_some_and(|required| target.semantic_type != *required)
                    {
                        continue;
                    }
                    if edges.len() >= limits.max_semantic_edges {
                        return Err(AdapterError::ResourceLimit {
                            kind: "semantic edges",
                            maximum: limits.max_semantic_edges as u64,
                        });
                    }
                    let id = hash_value(&SemanticEdgeIdentity {
                        schema_version: "qbm.semantic-edge-identity/v1",
                        adapter_id: &package.manifest.id,
                        package_hash,
                        mapping_rule_id: &rule.id,
                        relation_type: &rule.relation_type,
                        from: &source.id,
                        to: &target.id,
                        source_edge: &raw_edge.id,
                    })?;
                    let edge = SemanticEdge {
                        id,
                        adapter_id: package.manifest.id.clone(),
                        package_hash: package_hash.clone(),
                        mapping_rule_id: rule.id.clone(),
                        from: source.id.clone(),
                        to: target.id.clone(),
                        relation_type: rule.relation_type.clone(),
                        source_edge: raw_edge.id.clone(),
                        evidence: vec![raw_edge.evidence.clone()],
                    };
                    charge_semantic_output(output_budget, &edge, limits)?;
                    edges.push(edge);
                }
            }
        }
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(ProjectedPackage { nodes, edges })
}

fn mapping_value<F>(
    node: &GraphNode,
    source: &MappingValueSource,
    evidence_value: &mut F,
) -> Result<Option<Value>, AdapterError>
where
    F: FnMut(&GraphNode) -> Result<Option<Value>, AdapterError>,
{
    match source {
        MappingValueSource::Identifier => Ok(node.identifier.as_ref().map(|value| json!(value))),
        MappingValueSource::Label => Ok(Some(json!(node.label))),
        MappingValueSource::RelativePath => Ok(node
            .evidence
            .relative_path
            .as_ref()
            .map(|value| json!(value))),
        MappingValueSource::Pointer => node
            .evidence
            .pointer
            .as_ref()
            .map(|value| json!(value))
            .map_or(Ok(None), |value| Ok(Some(value))),
        MappingValueSource::EvidenceValue => evidence_value(node),
        MappingValueSource::RawProperty { key } => Ok(node.properties.get(key).cloned()),
        MappingValueSource::Literal { value } => Ok(Some(value.clone())),
    }
}

fn semantic_external_id(node: &GraphNode, source: SemanticIdentitySource) -> String {
    match source {
        SemanticIdentitySource::RawNodeId => node.id.to_string(),
        SemanticIdentitySource::Identifier => node
            .identifier
            .clone()
            .unwrap_or_else(|| node.id.to_string()),
        SemanticIdentitySource::Label => node.label.clone(),
        SemanticIdentitySource::SourceLocation => node.evidence.relative_path.as_ref().map_or_else(
            || node.id.to_string(),
            |path| {
                node.evidence
                    .pointer
                    .as_ref()
                    .map_or_else(|| path.clone(), |pointer| format!("{path}#{pointer}"))
            },
        ),
    }
}

fn append_bounded<T>(
    target: &mut Vec<T>,
    values: Vec<T>,
    maximum: usize,
    kind: &'static str,
) -> Result<(), AdapterError> {
    if target.len().saturating_add(values.len()) > maximum {
        return Err(AdapterError::ResourceLimit {
            kind,
            maximum: maximum as u64,
        });
    }
    target.extend(values);
    Ok(())
}

fn charge_semantic_output<T: Serialize>(
    budget: &mut OutputBudget,
    value: &T,
    limits: AdapterLimits,
) -> Result<(), AdapterError> {
    let bytes = qbm_canonical::canonical_json(value)?.len();
    budget.semantic_bytes = budget.semantic_bytes.saturating_add(bytes);
    if budget.semantic_bytes > limits.max_total_semantic_output_bytes {
        return Err(AdapterError::ResourceLimit {
            kind: "cumulative semantic output bytes",
            maximum: limits.max_total_semantic_output_bytes as u64,
        });
    }
    Ok(())
}

fn reject_duplicate_ids<'a>(
    ids: impl Iterator<Item = &'a Sha256Digest>,
    kind: &'static str,
) -> Result<(), AdapterError> {
    let mut unique = BTreeSet::new();
    for id in ids {
        if !unique.insert(id) {
            return Err(AdapterError::InvalidRelationship(format!(
                "duplicate {kind} identity {id}"
            )));
        }
    }
    Ok(())
}

fn parse_document(
    source: &str,
    format: SourceFormat,
    limits: AdapterLimits,
) -> Result<ParsedDocument, AdapterError> {
    let document = match format {
        SourceFormat::Json => serde_json::from_str(source)
            .map(ParsedDocument::Single)
            .map_err(|error| AdapterError::Evidence(format!("JSON parse failed: {error}")))?,
        SourceFormat::Yaml => {
            let mut documents = Vec::new();
            for value in serde_yaml::Deserializer::from_str(source) {
                let value = Value::deserialize(value).map_err(|error| {
                    AdapterError::Evidence(format!("YAML parse failed: {error}"))
                })?;
                documents.push(value);
            }
            ParsedDocument::Yaml(documents)
        }
        SourceFormat::Toml => {
            let value: toml::Value = toml::from_str(source)
                .map_err(|error| AdapterError::Evidence(format!("TOML parse failed: {error}")))?;
            ParsedDocument::Single(serde_json::to_value(value)?)
        }
        SourceFormat::Rust | SourceFormat::Python | SourceFormat::Unknown => {
            ParsedDocument::Unsupported
        }
    };
    match &document {
        ParsedDocument::Single(value) => validate_value_budget(value, limits)?,
        ParsedDocument::Yaml(values) => {
            let mut elements = 0_usize;
            for value in values {
                elements = elements.saturating_add(value_budget(value, 0, limits)?);
                if elements > limits.max_value_elements {
                    return Err(AdapterError::ResourceLimit {
                        kind: "evidence value elements",
                        maximum: limits.max_value_elements as u64,
                    });
                }
            }
        }
        ParsedDocument::Unsupported => {}
    }
    Ok(document)
}

fn validate_value_budget(value: &Value, limits: AdapterLimits) -> Result<(), AdapterError> {
    let _ = value_budget(value, 0, limits)?;
    Ok(())
}

fn value_budget(value: &Value, depth: usize, limits: AdapterLimits) -> Result<usize, AdapterError> {
    if depth > limits.max_value_depth {
        return Err(AdapterError::ResourceLimit {
            kind: "evidence value depth",
            maximum: limits.max_value_depth as u64,
        });
    }
    let mut elements = 1_usize;
    match value {
        Value::Array(values) => {
            for value in values {
                elements = elements.saturating_add(value_budget(value, depth + 1, limits)?);
                if elements > limits.max_value_elements {
                    break;
                }
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                elements = elements.saturating_add(value_budget(value, depth + 1, limits)?);
                if elements > limits.max_value_elements {
                    break;
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    if elements > limits.max_value_elements {
        return Err(AdapterError::ResourceLimit {
            kind: "evidence value elements",
            maximum: limits.max_value_elements as u64,
        });
    }
    Ok(elements)
}

fn document_value(document: &ParsedDocument, pointer: Option<&str>) -> Option<Value> {
    let pointer = pointer.unwrap_or_default();
    match document {
        ParsedDocument::Single(value) => value.pointer(pointer).cloned(),
        ParsedDocument::Yaml(documents) => {
            if let Some(remainder) = pointer.strip_prefix("/documents/") {
                let (index, nested) = remainder
                    .split_once('/')
                    .map_or((remainder, ""), |(index, nested)| (index, nested));
                let index = index.parse::<usize>().ok()?;
                let nested_pointer = if nested.is_empty() {
                    String::new()
                } else {
                    format!("/{nested}")
                };
                documents.get(index)?.pointer(&nested_pointer).cloned()
            } else {
                documents.first()?.pointer(pointer).cloned()
            }
        }
        ParsedDocument::Unsupported => None,
    }
}

#[allow(clippy::too_many_lines)]
fn audit_package(
    graph: &SemanticGraph,
    package: &AdapterPackage,
    package_hash: &Sha256Digest,
    maximum_findings: usize,
    fuel: &mut Fuel,
    audit_budget: &mut AuditBudget,
    limits: AdapterLimits,
) -> Result<Vec<AuditFinding>, AdapterError> {
    let nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.adapter_id == package.manifest.id && node.package_hash == *package_hash)
        .collect();
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.adapter_id == package.manifest.id && edge.package_hash == *package_hash)
        .collect();
    let mut rules: Vec<_> = package.policies.rules.iter().collect();
    rules.sort_by(|left, right| left.id.cmp(&right.id));
    let mut findings = Vec::new();
    for rule in rules {
        match &rule.assertion {
            SemanticPolicyAssertion::MinimumTypeCount {
                semantic_type,
                minimum,
            } => {
                let matching = matching_nodes(&nodes, semantic_type, fuel)?;
                if (matching.len() as u64) < *minimum {
                    push_finding(
                        &mut findings,
                        maximum_findings,
                        policy_finding(
                            package,
                            rule,
                            format!(
                                "Expected at least {minimum} {semantic_type:?} node(s), found {}.",
                                matching.len()
                            ),
                            matching,
                            Vec::new(),
                            limits,
                        )?,
                        audit_budget,
                        limits,
                    )?;
                }
            }
            SemanticPolicyAssertion::MaximumTypeCount {
                semantic_type,
                maximum,
            } => {
                let matching = matching_nodes(&nodes, semantic_type, fuel)?;
                if matching.len() as u64 > *maximum {
                    push_finding(
                        &mut findings,
                        maximum_findings,
                        policy_finding(
                            package,
                            rule,
                            format!(
                                "Expected at most {maximum} {semantic_type:?} node(s), found {}.",
                                matching.len()
                            ),
                            matching,
                            Vec::new(),
                            limits,
                        )?,
                        audit_budget,
                        limits,
                    )?;
                }
            }
            SemanticPolicyAssertion::RequiredAttribute {
                semantic_type,
                attribute,
            } => {
                for node in &nodes {
                    fuel.consume(1)?;
                    if node.semantic_type == *semantic_type
                        && !node.attributes.contains_key(attribute)
                    {
                        push_finding(
                            &mut findings,
                            maximum_findings,
                            policy_finding(
                                package,
                                rule,
                                format!(
                                    "Node {} of type {semantic_type:?} lacks required attribute {attribute:?}.",
                                    node.id
                                ),
                                vec![*node],
                                Vec::new(),
                                limits,
                            )?,
                            audit_budget,
                            limits,
                        )?;
                    }
                }
            }
            SemanticPolicyAssertion::UniqueExternalId { semantic_type } => {
                let mut grouped: BTreeMap<&str, Vec<&SemanticNode>> = BTreeMap::new();
                for node in &nodes {
                    fuel.consume(1)?;
                    if node.semantic_type == *semantic_type {
                        grouped.entry(&node.external_id).or_default().push(node);
                    }
                }
                for (external_id, duplicates) in grouped {
                    if duplicates.len() > 1 {
                        push_finding(
                            &mut findings,
                            maximum_findings,
                            policy_finding(
                                package,
                                rule,
                                format!(
                                    "External identity {external_id:?} occurs {} times for {semantic_type:?}.",
                                    duplicates.len()
                                ),
                                duplicates,
                                Vec::new(),
                                limits,
                            )?,
                            audit_budget,
                            limits,
                        )?;
                    }
                }
            }
            SemanticPolicyAssertion::RequiredOutgoingRelation {
                semantic_type,
                relation_type,
            } => {
                let mut sources = BTreeSet::new();
                for edge in &edges {
                    fuel.consume(1)?;
                    if edge.relation_type == *relation_type {
                        sources.insert(&edge.from);
                    }
                }
                for node in &nodes {
                    fuel.consume(1)?;
                    if node.semantic_type == *semantic_type && !sources.contains(&node.id) {
                        push_finding(
                            &mut findings,
                            maximum_findings,
                            policy_finding(
                                package,
                                rule,
                                format!(
                                    "Node {} of type {semantic_type:?} has no outgoing {relation_type:?} relationship.",
                                    node.id
                                ),
                                vec![*node],
                                Vec::new(),
                                limits,
                            )?,
                            audit_budget,
                            limits,
                        )?;
                    }
                }
            }
            SemanticPolicyAssertion::ForbidSelfRelation { relation_type } => {
                let by_id: BTreeMap<_, _> = nodes.iter().map(|node| (&node.id, *node)).collect();
                for edge in &edges {
                    fuel.consume(1)?;
                    if edge.relation_type == *relation_type && edge.from == edge.to {
                        let affected = by_id.get(&edge.from).copied().into_iter().collect();
                        push_finding(
                            &mut findings,
                            maximum_findings,
                            policy_finding(
                                package,
                                rule,
                                format!(
                                    "Relationship {} creates a forbidden {relation_type:?} self-loop.",
                                    edge.id
                                ),
                                affected,
                                edge.evidence.clone(),
                                limits,
                            )?,
                            audit_budget,
                            limits,
                        )?;
                    }
                }
            }
        }
    }
    sort_findings(&mut findings);
    Ok(findings)
}

fn matching_nodes<'a>(
    nodes: &[&'a SemanticNode],
    semantic_type: &str,
    fuel: &mut Fuel,
) -> Result<Vec<&'a SemanticNode>, AdapterError> {
    let mut matching = Vec::new();
    for node in nodes {
        fuel.consume(1)?;
        if node.semantic_type == semantic_type {
            matching.push(*node);
        }
    }
    Ok(matching)
}

fn policy_finding(
    package: &AdapterPackage,
    rule: &qbm_domain::SemanticPolicyRule,
    message: String,
    mut nodes: Vec<&SemanticNode>,
    mut extra_evidence: Vec<EvidenceLocation>,
    limits: AdapterLimits,
) -> Result<AuditFinding, AdapterError> {
    let code = format!("{}::{}", package.manifest.id, rule.id);
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    nodes.dedup_by(|left, right| left.id == right.id);
    nodes.truncate(limits.max_finding_affected_nodes);
    let affected_nodes: Vec<_> = nodes.iter().map(|node| node.id.clone()).collect();
    for node in nodes {
        extra_evidence.extend(node.evidence.clone());
    }
    extra_evidence.sort();
    extra_evidence.dedup();
    extra_evidence.truncate(limits.max_finding_evidence_locations);
    let id = hash_value(&FindingIdentity {
        schema_version: "qbm.semantic-finding-identity/v1",
        code: &code,
        severity: rule.severity,
        title: &rule.title,
        message: &message,
        remediation: &rule.remediation,
        affected_nodes: &affected_nodes,
        evidence: &extra_evidence,
    })?;
    Ok(AuditFinding {
        id,
        code,
        severity: rule.severity,
        title: rule.title.clone(),
        message,
        remediation: rule.remediation.clone(),
        affected_nodes,
        evidence: extra_evidence,
    })
}

fn push_finding(
    findings: &mut Vec<AuditFinding>,
    maximum: usize,
    finding: AuditFinding,
    budget: &mut AuditBudget,
    limits: AdapterLimits,
) -> Result<(), AdapterError> {
    if findings.len() >= maximum {
        return Err(AdapterError::ResourceLimit {
            kind: "semantic findings",
            maximum: maximum as u64,
        });
    }
    let bytes = qbm_canonical::canonical_json(&finding)?.len();
    budget.canonical_bytes = budget.canonical_bytes.saturating_add(bytes);
    if budget.canonical_bytes > limits.max_total_audit_output_bytes {
        return Err(AdapterError::ResourceLimit {
            kind: "cumulative semantic audit output bytes",
            maximum: limits.max_total_audit_output_bytes as u64,
        });
    }
    findings.push(finding);
    Ok(())
}

fn sort_findings(findings: &mut [AuditFinding]) {
    findings.sort_by(|left, right| {
        Reverse(left.severity)
            .cmp(&Reverse(right.severity))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.id.cmp(&right.id))
    });
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

fn resolve_package_projections(
    graph: &SemanticGraph,
    package: &AdapterPackage,
    package_hash: &Sha256Digest,
    fuel: &mut Fuel,
    budget: &mut CatalogBudget,
    limits: AdapterLimits,
) -> Result<Vec<ResolvedProjection>, AdapterError> {
    let package_nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.adapter_id == package.manifest.id && node.package_hash == *package_hash)
        .collect();
    let package_edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.adapter_id == package.manifest.id && edge.package_hash == *package_hash)
        .collect();
    let available_types: BTreeSet<_> = package_nodes
        .iter()
        .map(|node| node.semantic_type.as_str())
        .collect();
    let mut definitions: Vec<_> = package.projections.iter().collect();
    definitions.sort_by(|left, right| left.id.cmp(&right.id));
    let mut output = Vec::with_capacity(definitions.len());
    for definition in definitions {
        let mut unavailable_reasons = Vec::new();
        for semantic_type in &definition.semantic_types {
            fuel.consume(1)?;
            if !available_types.contains(semantic_type.as_str()) {
                unavailable_reasons.push(format!("semantic type {semantic_type:?} is absent"));
            }
        }
        let mut nodes = Vec::new();
        for node in &package_nodes {
            fuel.consume(1)?;
            if definition.semantic_types.is_empty()
                || definition.semantic_types.contains(&node.semantic_type)
            {
                charge_projection_member(budget, limits)?;
                nodes.push(node.id.clone());
            }
        }
        nodes.sort();
        let selected_nodes: BTreeSet<_> = nodes.iter().collect();
        let mut edges = Vec::new();
        let mut selected_relations = BTreeSet::new();
        for edge in &package_edges {
            fuel.consume(1)?;
            if selected_nodes.contains(&edge.from)
                && selected_nodes.contains(&edge.to)
                && (definition.relation_types.is_empty()
                    || definition.relation_types.contains(&edge.relation_type))
            {
                charge_projection_member(budget, limits)?;
                selected_relations.insert(edge.relation_type.as_str());
                edges.push(edge.id.clone());
            }
        }
        edges.sort();
        for relation_type in &definition.relation_types {
            fuel.consume(1)?;
            if !selected_relations.contains(relation_type.as_str()) {
                unavailable_reasons.push(format!(
                    "relationship {relation_type:?} is absent between selected nodes"
                ));
            }
        }
        unavailable_reasons.sort();
        let projection = ResolvedProjection {
            adapter_id: package.manifest.id.clone(),
            package_hash: package_hash.clone(),
            projection_id: definition.id.clone(),
            display_name: definition.display_name.clone(),
            description: definition.description.clone(),
            available: unavailable_reasons.is_empty(),
            unavailable_reasons,
            nodes,
            edges,
        };
        let bytes = qbm_canonical::canonical_json(&projection)?.len();
        budget.canonical_bytes = budget.canonical_bytes.saturating_add(bytes);
        if budget.canonical_bytes > limits.max_total_projection_output_bytes {
            return Err(AdapterError::ResourceLimit {
                kind: "cumulative projection catalog output bytes",
                maximum: limits.max_total_projection_output_bytes as u64,
            });
        }
        output.push(projection);
    }
    Ok(output)
}

fn charge_projection_member(
    budget: &mut CatalogBudget,
    limits: AdapterLimits,
) -> Result<(), AdapterError> {
    budget.members = budget.members.saturating_add(1);
    if budget.members > limits.max_total_projection_members {
        return Err(AdapterError::ResourceLimit {
            kind: "projection catalog memberships",
            maximum: limits.max_total_projection_members as u64,
        });
    }
    Ok(())
}
