//! Integration tests for conservative NSCLC-style profile projection.

use qbm_benchmark::{
    BENCHMARK_PROFILE_SCHEMA_VERSION, BenchmarkConstraints, BenchmarkInput, BenchmarkObjective,
    BenchmarkOutcome, BenchmarkProfile, BiomedicalScope, IncidenceRelationship, ProfileProvenance,
};
use qbm_profiler::{
    DiagnosticSeverity, Profiler, ProfilerError, ProfilerLimits, ProjectionCandidate,
    ProjectionContext, ProjectionMethod, ProjectionStatus, SourceDocument, project_documents,
};
use serde_json::{Value, json};

fn yaml_document(path: &str, artifact_id: &str, yaml: &str) -> SourceDocument {
    SourceDocument::new(
        path,
        artifact_id,
        serde_yaml::from_str::<Value>(yaml).expect("valid test YAML"),
    )
}

fn explicit_profile(artifact_id: &str) -> BenchmarkProfile {
    BenchmarkProfile {
        schema_version: BENCHMARK_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id: "nsclc-panel".to_owned(),
        title: "Reviewed NSCLC panel".to_owned(),
        biomedical_scope: BiomedicalScope {
            area: "oncology".to_owned(),
            population: "advanced NSCLC".to_owned(),
            input_semantics: "biomarker assay".to_owned(),
            outcome_semantics: "therapy option".to_owned(),
        },
        inputs: vec![BenchmarkInput {
            id: "EGFR".to_owned(),
            label: "EGFR alteration".to_owned(),
            cost: 2.5,
            tags: vec!["genomic".to_owned()],
        }],
        outcomes: vec![BenchmarkOutcome {
            id: "osimertinib".to_owned(),
            label: "Osimertinib".to_owned(),
            weight: 3.0,
            tags: vec!["therapy".to_owned()],
        }],
        relationships: vec![IncidenceRelationship::supporting(
            "EGFR".to_owned(),
            "osimertinib".to_owned(),
        )],
        unconditional_outcomes: Vec::new(),
        constraints: BenchmarkConstraints {
            min_selected: 0,
            max_selected: Some(1),
            max_total_cost: None,
            required_inputs: Vec::new(),
            excluded_inputs: Vec::new(),
            required_outcomes: Vec::new(),
        },
        objective: BenchmarkObjective::MaximizeWeightedCoverage,
        provenance: ProfileProvenance {
            generated_by: "curator/1".to_owned(),
            source_revision: Some("study-v3".to_owned()),
            source_artifact_ids: vec![artifact_id.to_owned()],
            projection_method: "reviewed canonical profile".to_owned(),
        },
    }
}

#[test]
fn projects_nsclc_yaml_with_evidence_and_explicit_defaults() {
    let document = yaml_document(
        "clinical/nsclc.yaml",
        "artifact-nsclc-v1",
        r"
profile_id: nsclc-panel
title: NSCLC precision oncology panel
domain: oncology
indication: NSCLC
biomarkers:
  - biomarker_id: EGFR
    label: EGFR alteration
    cost: 2.5
    tags: [genomic, actionable]
  - biomarker_id: ALK
    label: ALK fusion
actions:
  - action_id: osimertinib
    label: Osimertinib
    weight: 3
  - option_id: alk_inhibitor
    label: ALK inhibitor option
rules:
  - conditions:
      - field: EGFR
        equals: positive
    action: osimertinib
  - conditions:
      - input_id: ALK
        equals: positive
    outcome: alk_inhibitor
",
    );

    let candidate = project_documents(&[document]).expect("reviewable projection");

    assert_eq!(candidate.method, ProjectionMethod::BiomedicalHeuristic);
    assert_eq!(candidate.status, ProjectionStatus::RequiresApproval);
    assert!(candidate.confidence_bps < 10_000);
    assert_eq!(candidate.profile.profile_id, "nsclc-panel");
    assert_eq!(candidate.profile.biomedical_scope.area, "oncology");
    assert_eq!(candidate.profile.biomedical_scope.population, "NSCLC");
    assert_eq!(
        candidate
            .profile
            .inputs
            .iter()
            .map(|input| (input.id.as_str(), input.cost))
            .collect::<Vec<_>>(),
        vec![("ALK", 1.0), ("EGFR", 2.5)]
    );
    assert_eq!(
        candidate
            .profile
            .outcomes
            .iter()
            .map(|outcome| (outcome.id.as_str(), outcome.weight))
            .collect::<Vec<_>>(),
        vec![("alk_inhibitor", 1.0), ("osimertinib", 3.0)]
    );
    assert_eq!(
        candidate.profile.relationships,
        vec![
            IncidenceRelationship::supporting("ALK".to_owned(), "alk_inhibitor".to_owned()),
            IncidenceRelationship::supporting("EGFR".to_owned(), "osimertinib".to_owned()),
        ]
    );
    assert!(candidate.assumptions.iter().any(|assumption| {
        assumption.code == "default_input_cost"
            && assumption.target_field == "/inputs/ALK/cost"
            && assumption.value == "1"
    }));
    assert!(candidate.assumptions.iter().any(|assumption| {
        assumption.code == "default_outcome_weight"
            && assumption.target_field == "/outcomes/alk_inhibitor/weight"
            && assumption.value == "1"
    }));
    assert_eq!(
        candidate.profile.provenance.source_artifact_ids,
        vec!["artifact-nsclc-v1"]
    );
    assert!(candidate.evidence.iter().any(|evidence| {
        evidence.artifact_id == "artifact-nsclc-v1"
            && evidence.path == "clinical/nsclc.yaml"
            && evidence.pointer == "/rules/0/conditions/0/field"
    }));
    candidate.validate().expect("candidate invariants");
}

#[test]
fn projects_bounded_string_condition_atoms_without_interpreting_values() {
    let document = yaml_document(
        "clinical/decision_rules.yaml",
        "artifact-string-conditions",
        r"
domain: oncology
rules:
  - conditions:
      all_of:
        - ADVANCED_DISEASE_FLAG
        - STAGE_GROUP == STAGE_IIIA
        - field: context.DRIVER
          operator: IN
          values: [EGFR_SENSITISING, ALK_POSITIVE]
        - not: EXCLUSION_FLAG
    outcome: ADVANCED_DRIVER_TARGETED_THERAPY
",
    );

    let candidate = project_documents(&[document]).expect("reviewable projection");

    assert_eq!(
        candidate
            .profile
            .inputs
            .iter()
            .map(|input| input.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "ADVANCED_DISEASE_FLAG",
            "EXCLUSION_FLAG",
            "STAGE_GROUP",
            "context.DRIVER",
        ]
    );
    assert!(
        !candidate
            .profile
            .inputs
            .iter()
            .any(|input| input.id == "EGFR_SENSITISING" || input.id == "ALK_POSITIVE")
    );
    // The rule's three positive conditions hold together, so they are one arm
    // of three members rather than three independent edges. EXCLUSION_FLAG sits
    // under `not`, which is satisfied when that input is absent, so it creates
    // no requirement and no relationship.
    assert_eq!(candidate.profile.relationships.len(), 3);
    let paths: Vec<_> = candidate
        .profile
        .relationships
        .iter()
        .map(|relationship| relationship.path.as_str())
        .collect();
    assert!(
        paths.iter().all(|path| !path.is_empty()) && paths.windows(2).all(|w| w[0] == w[1]),
        "the three conditions should share one arm, got {paths:?}"
    );
    assert!(
        !candidate
            .profile
            .relationships
            .iter()
            .any(|relationship| relationship.input_id == "EXCLUSION_FLAG"),
        "a negated guard must not become a reason to select its input"
    );
    assert!(
        candidate
            .profile
            .inputs
            .iter()
            .any(|input| input.id == "EXCLUSION_FLAG"),
        "the negated input is still declared, and will read as inert"
    );
    assert!(
        candidate
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "negated_condition_creates_no_requirement" })
    );
    assert!(candidate.evidence.iter().any(|evidence| {
        evidence.path == "clinical/decision_rules.yaml"
            && evidence.pointer == "/rules/0/conditions/all_of/1"
            && evidence
                .target_field
                .starts_with("/relationships/STAGE_GROUP->ADVANCED_DRIVER_TARGETED_THERAPY")
    }));
    candidate.validate().expect("candidate invariants");
}

#[test]
fn projection_is_deterministic_across_document_order_and_deduplicates() {
    let declarations = SourceDocument::new(
        "model/entities.json",
        "artifact-entities",
        json!({
            "domain": "precision oncology",
            "indication": "advanced NSCLC",
            "inputs": [
                {"parameter_id": "PDL1", "label": "PD-L1", "cost": 4.0},
                {"feature_id": "EGFR", "label": "EGFR", "cost": 2.0}
            ],
            "outcomes": [
                {"outcome_id": "targeted_therapy", "weight": 2.0},
                {"option_id": "immunotherapy", "weight": 1.5}
            ]
        }),
    );
    let rules = yaml_document(
        "model/rules.yaml",
        "artifact-rules",
        r"
rules:
  - conditions:
      - parameter_id: PDL1
      - input: EGFR
    then:
      action_id: immunotherapy
      outcome: targeted_therapy
  - conditions:
      - field: EGFR
    action: targeted_therapy
",
    );
    let context = ProjectionContext {
        profile_id: Some("project-42-nsclc".to_owned()),
        title: Some("Stable NSCLC benchmark".to_owned()),
        biomedical_area: Some("oncology".to_owned()),
        population: Some("advanced NSCLC".to_owned()),
        source_revision: Some("commit-abc".to_owned()),
        generated_by: Some("qbm-web/0.1".to_owned()),
    };
    let profiler = Profiler::new();
    let forward = profiler
        .project_with_context(&[declarations.clone(), rules.clone()], &context)
        .expect("forward projection");
    let reverse = profiler
        .project_with_context(&[rules, declarations], &context)
        .expect("reverse projection");

    assert_eq!(forward, reverse);
    assert_eq!(forward.profile.profile_id, "project-42-nsclc");
    assert_eq!(
        forward.profile.provenance.source_revision.as_deref(),
        Some("commit-abc")
    );
    assert_eq!(forward.profile.provenance.generated_by, "qbm-web/0.1");
    assert_eq!(
        forward.profile.provenance.source_artifact_ids,
        vec!["artifact-entities", "artifact-rules"]
    );
    // Rule one needs PDL1 and EGFR together; rule two needs EGFR alone. Both
    // cover targeted_therapy, so it has two distinct arms — five relationships,
    // not four. Collapsing them would assert that EGFR alone satisfies the
    // first rule, which the document does not say.
    assert_eq!(forward.profile.relationships.len(), 5);
    let targeted_paths: Vec<_> = forward
        .profile
        .relationships
        .iter()
        .filter(|relationship| relationship.outcome_id == "targeted_therapy")
        .map(|relationship| relationship.path.as_str())
        .collect();
    assert_eq!(
        targeted_paths
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2,
        "targeted_therapy should keep two ways of being covered, got {targeted_paths:?}"
    );
}

#[test]
fn caller_profile_id_is_stable_when_artifact_content_identity_changes() {
    let context = ProjectionContext {
        profile_id: Some("logical-project/nsclc".to_owned()),
        ..ProjectionContext::default()
    };
    let value = json!({
        "domain": "oncology",
        "biomarker_id": "EGFR",
        "rules": [{"conditions": [{"field": "EGFR"}], "action": "therapy"}]
    });
    let first = Profiler::new()
        .project_with_context(
            &[SourceDocument::new(
                "model/profile.json",
                "artifact-version-one",
                value.clone(),
            )],
            &context,
        )
        .expect("first projection");
    let second = Profiler::new()
        .project_with_context(
            &[SourceDocument::new(
                "model/profile.json",
                "artifact-version-two",
                value,
            )],
            &context,
        )
        .expect("second projection");

    assert_eq!(first.profile.profile_id, second.profile.profile_id);
    assert_ne!(
        first.profile.provenance.source_artifact_ids,
        second.profile.provenance.source_artifact_ids
    );
}

#[test]
fn explicit_canonical_profile_is_validated_and_evidence_linked() {
    let artifact_id = "canonical-profile-artifact";
    let profile = explicit_profile(artifact_id);
    let document = SourceDocument::new(
        "profiles/qbm.profile.json",
        artifact_id,
        serde_json::to_value(&profile).expect("serializable profile"),
    );

    let candidate = project_documents(&[document]).expect("explicit projection");

    assert_eq!(candidate.method, ProjectionMethod::ExplicitProfile);
    assert_eq!(candidate.profile, profile);
    assert_eq!(candidate.confidence_bps, 10_000);
    assert!(candidate.defaulted_fields.is_empty());
    assert!(candidate.assumptions.is_empty());
    assert!(
        candidate
            .extracted_fields
            .contains(&"/inputs/0/id".to_owned())
    );
    assert!(candidate.evidence.iter().any(|evidence| {
        evidence.path == "profiles/qbm.profile.json"
            && evidence.pointer == "/relationships/0/input_id"
            && evidence.target_field == "/relationships/0/input_id"
    }));
}

#[test]
fn qbm_profile_envelope_is_supported_and_adds_its_artifact_to_provenance() {
    let profile = explicit_profile("underlying-study");
    let document = SourceDocument::new(
        "exports/reviewed.yaml",
        "envelope-artifact",
        json!({"qbm-profile": profile}),
    );

    let candidate = project_documents(&[document]).expect("enveloped profile");

    assert_eq!(candidate.method, ProjectionMethod::ExplicitProfile);
    assert_eq!(
        candidate.profile.provenance.source_artifact_ids,
        vec!["envelope-artifact", "underlying-study"]
    );
    assert!(candidate.assumptions.iter().any(|assumption| {
        assumption.code == "explicit_profile_artifact_provenance"
            && assumption.value == "envelope-artifact"
    }));
}

#[test]
fn rejects_non_biomedical_structured_shapes_and_source_language_names() {
    let generic = SourceDocument::new(
        "config/routing.json",
        "generic-artifact",
        json!({
            "input_id": "request",
            "outcome_id": "response",
            "rules": [{"conditions": [{"field": "request"}], "action": "response"}]
        }),
    );
    assert!(matches!(
        project_documents(&[generic]),
        Err(ProfilerError::NoProjection(_))
    ));

    let source_names = SourceDocument::new(
        "src/EGFR_to_osimertinib.rs",
        "source-artifact",
        json!({
            "domain": "oncology",
            "function_name": "EGFR_positive",
            "return_type": "osimertinib"
        }),
    );
    assert!(matches!(
        project_documents(&[source_names]),
        Err(ProfilerError::NoProjection(_))
    ));
}

#[test]
fn conflicting_values_are_selected_by_evidence_order_and_diagnosed() {
    let later = SourceDocument::new(
        "z/entities.json",
        "z-artifact",
        json!({
            "domain": "oncology",
            "biomarker_id": "EGFR",
            "label": "EGFR later",
            "cost": 9.0,
            "action_id": "therapy",
            "weight": 4.0
        }),
    );
    let earlier = SourceDocument::new(
        "a/model.json",
        "a-artifact",
        json!({
            "domain": "oncology",
            "biomarker_id": "EGFR",
            "label": "EGFR selected",
            "cost": 2.0,
            "action_id": "therapy",
            "weight": 3.0,
            "rules": [
                {"conditions": [{"field": "EGFR"}], "action": "therapy"},
                {"conditions": [{"field": "EGFR"}], "action": "therapy"}
            ]
        }),
    );

    let candidate = project_documents(&[later, earlier]).expect("deduplicated projection");

    assert_eq!(candidate.profile.inputs.len(), 1);
    assert_eq!(candidate.profile.inputs[0].label, "EGFR selected");
    assert!((candidate.profile.inputs[0].cost - 2.0).abs() < f64::EPSILON);
    assert!((candidate.profile.outcomes[0].weight - 3.0).abs() < f64::EPSILON);
    assert_eq!(candidate.profile.relationships.len(), 1);
    assert!(candidate.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "conflicting_entity_number"
            && diagnostic.severity == DiagnosticSeverity::Warning
    }));
}

#[test]
fn explicit_profile_must_obey_benchmark_sorting_and_references() {
    let artifact_id = "bad-profile-artifact";
    let mut profile = explicit_profile(artifact_id);
    profile.inputs.push(BenchmarkInput {
        id: "ALK".to_owned(),
        label: "ALK".to_owned(),
        cost: 1.0,
        tags: Vec::new(),
    });
    let document = SourceDocument::new(
        "qbm-profile.json",
        artifact_id,
        serde_json::to_value(profile).expect("serializable invalid profile"),
    );

    assert!(matches!(
        project_documents(&[document]),
        Err(ProfilerError::InvalidProfile(_))
    ));
}

#[test]
fn input_depth_and_document_count_are_bounded() {
    let limits = ProfilerLimits {
        max_documents: 1,
        max_value_depth: 2,
        ..ProfilerLimits::default()
    };
    let profiler = Profiler::with_limits(limits);
    let first = SourceDocument::new(
        "one.json",
        "artifact-one",
        json!({"domain": {"nested": "oncology"}}),
    );
    let second = SourceDocument::new("two.json", "artifact-two", json!({}));
    assert!(matches!(
        profiler.project(&[first.clone(), second]),
        Err(ProfilerError::ResourceLimit {
            kind: "source documents",
            maximum: 1
        })
    ));
    assert!(matches!(
        profiler.project(&[first]),
        Err(ProfilerError::ResourceLimit {
            kind: "value depth",
            maximum: 2
        })
    ));
}

#[test]
fn projection_candidate_round_trips_and_revalidates() {
    let document = SourceDocument::new(
        "model/nsclc.json",
        "roundtrip-artifact",
        json!({
            "domain": "oncology",
            "feature_id": "KRAS",
            "rules": [{
                "conditions": [{"input": "KRAS"}],
                "outcome": "kras_pathway"
            }]
        }),
    );
    let candidate = project_documents(&[document]).expect("projection");
    let encoded = serde_json::to_vec(&candidate).expect("serialize candidate");
    let decoded: ProjectionCandidate =
        serde_json::from_slice(&encoded).expect("deserialize candidate");

    assert_eq!(candidate, decoded);
    decoded.validate().expect("round-tripped candidate");
}
