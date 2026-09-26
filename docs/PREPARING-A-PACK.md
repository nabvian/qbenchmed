# Preparing your project for Q-BenchMed

Q-BenchMed does not know anything about your domain. It reads structure. This
document explains what it looks for, why it refuses a project it cannot read,
and the three ways to give it what it needs.

## What it is actually looking for

The engine optimises one thing: pick a set of inputs that covers as much
declared outcome weight as possible. To do that it needs three lists and the
relationships between the first two.

**Inputs.** The things you can choose to acquire. Biomarkers, assays, imaging
studies, questions on a form. Each has a stable identifier and a positive cost.

**Outcomes.** The things you want covered. Decisions a rule can reach,
phenotypes, classifications. Each has a stable identifier and a positive
importance weight.

**Relationships.** Which inputs let which outcomes fire. This is the part most
projects get wrong on the first attempt, and it is covered in its own section
below.

Nothing here is biomedical to the engine. The word biomarker appears in the
documentation because that is what the reference instance contains; the schema
calls them inputs and will take anything.

## Three ways to supply it

### 1. Export a profile

The most reliable option and the only one that never guesses. Write a
`qbm.profile.json` or `qbm.profile.yaml` at your project root, conforming to
`qbm.benchmark-profile/v2`. The Platform reads it verbatim.

A minimal one:

```json
{
  "schema_version": "qbm.benchmark-profile/v2",
  "profile_id": "biomedical/my-panel",
  "title": "My panel",
  "biomedical_scope": {
    "area": "haematology",
    "population": "Adult outpatient cohort",
    "input_semantics": "A selectable biomarker",
    "outcome_semantics": "A decision the rules can reach"
  },
  "inputs": [
    {"id": "HGB", "label": "Haemoglobin", "cost": 1.0, "tags": []},
    {"id": "MCV", "label": "Mean cell volume", "cost": 1.0, "tags": []}
  ],
  "outcomes": [
    {"id": "MICROCYTIC", "label": "Microcytic picture", "weight": 1.0, "tags": []}
  ],
  "relationships": [
    {"input_id": "HGB", "outcome_id": "MICROCYTIC", "path": "arm1"},
    {"input_id": "MCV", "outcome_id": "MICROCYTIC", "path": "arm1"}
  ],
  "constraints": {
    "min_selected": 0, "max_selected": null, "max_total_cost": null,
    "required_inputs": [], "excluded_inputs": [], "required_outcomes": []
  },
  "objective": "maximize_weighted_coverage",
  "provenance": {
    "generated_by": "my-exporter/v1",
    "source_revision": "abc123",
    "source_artifact_ids": ["my-project:v1"],
    "projection_method": "direct export from the rule table"
  }
}
```

The schema is strict on purpose. Inputs, outcomes, relationships, tags and
constraint lists must be sorted and unique. Costs and weights must be finite
and positive. Dangling references and unknown fields are rejected rather than
ignored. Validate it before you commit it:

```bash
qbm bench check qbm.profile.json
```

If your project already has a structured rule table, writing an exporter is
usually an afternoon. `qbm/export.py` in this repository is a worked example:
it projects the Python package's own domain-profile type onto this schema, and
refuses rather than degrading when it meets something the schema cannot carry.

### 2. Let the built-in profiler read your rules

If your knowledge lives in JSON or YAML with a recognisable rule shape, the
Platform can derive a profile without an exporter. It reads two shapes.

An explicit rule list:

```yaml
domain: haematology
rules:
  - conditions:
      all_of:
        - field: MCV
        - field: RBC
    outcome: THALASSAEMIA_TRIGGER
```

Or a named condition group, where the outcome is the key:

```yaml
domain: haematology
normalized_flags:
  THALASSAEMIA_TRIGGER:
    all_of:
      - MCV_CLASS == MCV_MICRO
      - RBC > 5.0
```

Both project identically. The second form is common in real knowledge bundles
and was invisible to earlier versions of the profiler.

The result is a candidate that requires review, carries a confidence figure,
and records the evidence for every field it extracted. Treat it as a starting
point, not a result. Confidence on inferred profiles sits well below an
explicit export.

### 3. Write an adapter package

For a project family you will profile repeatedly, a declarative adapter gives
exact control over the mapping without writing code that the Platform executes.
See `Platform/docs/ADAPTER_AUTHORING.md`. Adapters are data only: they get no
process, network, filesystem or language execution capability.

## Declare your domain, or nothing happens

This is the single most common reason a project produces no profile.

The profiler will not treat arbitrary structured data as clinical decision
logic. It requires the project to say, in a machine-readable field, what domain
it belongs to. The accepted keys are:

```
area   biomedical_area   cancer_type   clinical_domain   disease
domain   indication   population   therapeutic_area
```

with a value the vocabulary recognises. The vocabulary covers laboratory and
pathology, imaging, the clinical specialties, molecular disciplines and
oncology. It is matched as a substring, so `haematology`, `clinical pathology`
and `paediatric cardiology` all pass.

In practice this is two lines:

```yaml
domain: haematology
population: Adult CBC decision support
```

Run Q-BenchMed across a set of real projects and most of the ones that produce
nothing are missing exactly this. A project with hundreds of well-formed rule
files will return no profile until it says what it is about. The gate exists
because without it a routing config or a retry policy is indistinguishable from
a clinical rule set, and the engine would happily optimise either.

## Conjunctions are the part that matters

A rule's conditions hold together. If a rule fires when MCV is low **and** RBC
is high, then that outcome needs both biomarkers, and neither one alone buys
anything.

Say that with a shared `path`. Relationships sharing an outcome and a non-empty
path form one arm, and every member of an arm must be selected for the arm to
fire. An outcome is covered when any one of its arms fires.

```json
{"input_id": "MCV", "outcome_id": "THAL", "path": "arm1"},
{"input_id": "RBC", "outcome_id": "THAL", "path": "arm1"}
```

Two rules reaching the same outcome are two arms, and either is enough:

```json
{"input_id": "MCV",      "outcome_id": "THAL", "path": "arm1"},
{"input_id": "RBC",      "outcome_id": "THAL", "path": "arm1"},
{"input_id": "HBA2",     "outcome_id": "THAL", "path": "arm2"}
```

Leaving the path empty means the edge is its own single-member arm, which is
the plain "any one of these covers it" reading. A profile with no paths anywhere
behaves exactly like the older binary schema, which is why `v1` documents still
load unchanged.

Getting this wrong is not a rounding error. Flattening a two-input rule into two
independent edges asserts that either input alone is sufficient. It removes the
property that makes coverage non-submodular, it stops outcomes being
unreachable that genuinely are, and it makes greedy selection look optimal on an
instance where it is not. On the reference instance, 86 of 116 arms need more
than one biomarker; flattening them produces an easier problem than the one the
rules describe.

Four other relationship kinds are available when you need them. `required`
gates an outcome on an input being present. `exclusionary` blocks an outcome
when an input is selected. `optional` records a weaker contribution.
`contextual` needs a second input present alongside the first. All are
documented in the schema.

## Reading the result

Run it:

```bash
qbm --data-dir .qbenchmed run express --source /path/to/project
```

Two labels in the output are worth understanding before you draw conclusions.

**Inert** means no relationship mentions that input at all. **Veto-only** means
every mention of it either blocks an outcome or supplies context, so selecting
it can never grant coverage. Both are unwired, in different ways.

**Unreachable** means no selection of any size can cover that outcome. Under
conjunctive semantics this can happen even when relationships mention it, if its
arms can never be satisfied together. That is a finding about your rule set, not
a bug in the tool.

None of these labels prove anything about your project outside the profile. They
describe the document the engine was given. If your exporter dropped something,
the tool reports the consequence of that, not the omission.

## When intake refuses

Real projects carry datasets, model weights and installers next to their
knowledge files. Intake counts cumulative bytes, so one unexcluded data
directory can push every later file past the ceiling and stop the run.

```bash
qbm run express --source /path/to/project \
    --exclude-name .venv --exclude-name models --exclude-name Data
```

Exclusions are recorded in the inventory, so the report always states what was
left out.

A project graph above one million nodes is refused outright. Scope the intake
with exclusions rather than raising the ceiling.

## What the tool will not do for you

It will not infer biomedical meaning from a function or variable name. A symbol
called `EGFR_positive` in Rust or Python source is a symbol, not a biomarker.

It will not execute anything in your project. Source is read as data. Rules,
tests, build scripts, notebooks and models are never run.

It will not invent a profile when the evidence is missing. It records that the
profile, comparison, optimisation and QUBO stages were skipped, and why.

And it will not turn a coverage result into a clinical claim. A covering panel
solves a coverage objective over a rule graph. It does not follow that any
biomarker is clinically unnecessary, or that a smaller panel is safe.
