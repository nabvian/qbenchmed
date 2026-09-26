# Open-Source Product and Ecosystem Design

## 1. Meaning of “really universal”

Q-BenchMed is universal only if a person can install it, point it at an unfamiliar project, and receive a useful audit without writing Rust, Python, an adapter, or a manifest first.

Custom adapters must deepen the audit; they must not be the price of entry.

The product promise is:

```text
Install one local application
  -> select a folder or run qbm audit .
  -> receive an immediate generic audit
  -> optionally accept an automatically suggested project profile
  -> add or install an adapter for domain-aware audits and optimization
```

Open source does not mean that projects audited with Q-BenchMed become public. The application is local-first, telemetry-free by default, and able to audit private repositories without uploading them.

## 2. Four capability levels

### Level 0 — Unknown-project audit

Works for every accessible folder without configuration.

It provides:

- safe file inventory and content hashes;
- language, framework, build-system, model, database, API, and documentation detection;
- dependency manifests and lockfile inventory;
- raw code, module, API, schema, test, and artifact graphs where supported;
- duplicate files and identifiers;
- broken machine-readable references;
- cycles and disconnected components in discovered graphs;
- secret/sensitive-file warnings;
- licence and provenance inventory;
- large/binary/model artifact identity;
- Git/worktree state;
- test/build command candidates without executing them;
- comparison with an earlier generic snapshot;
- structured JSON, SARIF, Markdown, and HTML report.

The report states parser coverage and uncertainty. It must not pretend that unknown domain objects have been understood.

### Level 1 — Generated project profile

The setup wizard proposes `.qbench/project.yaml` using discovery evidence:

- include/exclude paths;
- likely build and test commands;
- source languages and frameworks;
- local data and model locations;
- candidate API contracts;
- probable adapter families;
- sensitivity and execution policies.

The user reviews the proposal and saves it locally. The project can now be audited consistently without repeating setup.

### Level 2 — Configuration adapter

Many projects should need only a mapping pack, not custom code.

The guided adapter builder lets users map:

```text
source object pattern -> semantic node kind
source reference      -> semantic edge kind
field/path             -> ID, version, cost, weight, scope, status
test command           -> behavior runner contract
graph subset           -> benchmark projection
```

It generates a versioned adapter package and conformance fixtures. Users may keep the adapter private or publish it.

### Level 3 — Code adapter/plugin

Complex systems use a built-in Rust adapter, sandboxed WASM plugin, or controlled process bridge. This level supports custom parsing, graph construction, policy checks, behavior runners, and projections.

The UI always identifies which level produced each result.

## 3. Zero-configuration command

The central adoption command is:

```bash
qbm audit .
```

Expected behavior:

1. Detect whether a local Q-BenchMed service is already running.
2. Create or reuse the local metadata/artifact store.
3. Inventory the current folder without modification.
4. Apply the built-in generic scanner set.
5. Detect installed compatible adapters.
6. Run a generic static audit.
7. Produce a terminal summary and local HTML report.
8. Explain which deeper checks require a profile, adapter, runner, or approval.

No account, cloud connection, API key, Python installation, database server, or manual manifest is required for this path.

## 4. Audit modes for different users

### Quick

For first-time exploration:

- inventory and static generic audit;
- no code execution;
- no network;
- one final confirmation before exporting outside the local store.

### Standard

For everyday project work:

- generic plus installed adapters;
- baseline diff;
- approval before behavior execution, network acquisition, optimization, and publication;
- local report automatically generated.

### Deep

For maintainers and researchers:

- all static scanners;
- declared behavior runners;
- performance and model checks;
- multiple projections and algorithm comparisons;
- higher resource limits with explicit visibility.

### Governed

For clinical, scientific, financial, or regulated work:

- manual approval at every material stage;
- role separation where configured;
- immutable approval history;
- strict external-export controls;
- no waiving of designated safety/privacy checks.

The mode controls workflow friction, not audit truth. A Quick audit never masquerades as a governed audit.

## 5. Beginner-first browser workflow

The browser UI opens with three actions:

```text
[Choose Project Folder]  [Upload Archive]  [Register Large/Local Project]
```

After selection it shows:

```text
Project detected: Rust + Python + YAML + ONNX
Recommended audit: Standard
Adapter matches:
  Generic software adapter       100%
  ML model adapter                91%
  ONCOVA adapter                   0%

Files included: 1,842
Files excluded:   73
Sensitive candidates: 2 — review required

[Review] [Start Safe Static Audit]
```

The results page uses plain-language sections:

- What was inspected?
- What is healthy?
- What is broken or missing?
- What changed?
- What could not be checked?
- What should I do next?
- What can Q-BenchMed optimize safely?

Expert graph, schema, solver, and QUBO details are available through progressive disclosure.

## 6. Guided adapter creation

The adapter wizard turns unresolved discovery into a finite checklist:

```text
1. Choose project purpose
2. Confirm source roots and exclusions
3. Confirm inputs, contexts, derived values, rules, outputs, models, and datasets
4. Resolve AND/OR/precedence/negation semantics
5. Select generic and domain policies
6. Define or import golden fixtures
7. Define safe benchmark projections
8. Run conformance tests
9. Approve adapter v1
10. Save privately or publish
```

Commands:

```bash
qbm adapter infer .
qbm adapter new my-project
qbm adapter edit my-project
qbm adapter test my-project
qbm adapter pack my-project
qbm adapter install ./my-project.qbm-plugin
qbm adapter publish my-project.qbm-plugin
```

Publishing is always a separate, explicit action. Inferred mappings remain local by default.

## 7. Plugin package

A portable plugin package contains:

```text
my-adapter/
├── qbm-plugin.toml
├── README.md
├── LICENSE
├── adapter/
│   ├── mappings.yaml              # configuration adapter, when sufficient
│   └── adapter.wasm               # optional code adapter
├── policies/
├── schemas/
├── projections/
├── report-sections/
├── fixtures/
├── conformance/
├── CHANGELOG.md
└── SHA256SUMS
```

Plugin manifest:

```toml
schema_version = "qbm.plugin/v1"
id = "org.example.oncova"
name = "ONCOVA Adapter"
version = "1.0.0"
license = "Apache-2.0"
kind = "adapter"
qbm_api = ">=1.0,<2.0"

[capabilities]
read_snapshot = true
network = false
execute = false
secrets = false

[entrypoints]
mapping = "adapter/mappings.yaml"
wasm = "adapter/adapter.wasm"
```

The installer displays publisher, licence, requested capabilities, compatible Q-BenchMed versions, package hash, signature state, and included policies before installation.

## 8. Decentralized community registry

The public registry is an index, not a mandatory hosted execution service.

It records:

- plugin ID, name, description, and domain;
- source repository;
- release artifact and checksum;
- publisher identity/signature;
- licence;
- supported Q-BenchMed and schema versions;
- capabilities;
- conformance status;
- maintained/deprecated/withdrawn status;
- security advisory links.

Users can install from:

- the official community index;
- a Git release;
- a local file;
- a private organizational registry;
- an OCI-compatible artifact registry in a later release.

Q-BenchMed continues to work with no registry connection.

## 9. Plugin quality tiers

```text
LOCAL
  Created by the user; not externally reviewed.

COMMUNITY
  Published with schema validation and basic conformance results.

VERIFIED
  Reproducible build, signed release, complete conformance suite, reviewed capabilities.

BUILT_IN
  Released and tested with the Q-BenchMed distribution.
```

“Verified” means software/package verification, not clinical or scientific endorsement.

## 10. Compatibility promise

The open-source ecosystem needs stable boundaries.

Version independently:

- CLI behavior;
- HTTP API;
- plugin protocol;
- project manifest;
- snapshot format;
- raw graph schema;
- semantic graph schema;
- projection/objective schema;
- finding/report schema;
- Python/process bridge protocol.

Rules:

- semantic versioning for releases and public protocols;
- deprecation period before removing public fields or commands;
- machine-readable compatibility declarations;
- migration tools for stored artifacts and manifests;
- original immutable artifacts retained through migration;
- capability negotiation for plugins;
- conformance fixtures published with every protocol version;
- long-term support policy documented after the first stable release.

## 11. Open-source repository structure

In addition to application code, the public repository includes:

```text
LICENSE
NOTICE
README.md
CONTRIBUTING.md
CODE_OF_CONDUCT.md
SECURITY.md
GOVERNANCE.md
SUPPORT.md
CHANGELOG.md
ROADMAP.md
RELEASE.md
CODEOWNERS
.github/
  ISSUE_TEMPLATE/
  PULL_REQUEST_TEMPLATE.md
  workflows/
rfcs/
adrs/
examples/
plugin-index/
```

Recommended licence: this blueprint originally proposed Apache License 2.0. The project ships under the GNU AGPL version 3 or later instead, so that a hosted derivative has to return its source. Every community plugin and fixture still declares its own SPDX licence; no incompatible or unknown licence is silently bundled.

Recommended contribution model: Developer Certificate of Origin sign-off, public review, required tests, and no mandatory copyright assignment.

## 12. Project governance

Roles:

- contributors;
- reviewers;
- component maintainers;
- release maintainers;
- security response team;
- technical steering maintainers for cross-cutting decisions.

Decision path:

- routine changes through pull requests;
- architectural decisions recorded as ADRs;
- public protocol, schema, governance, or security-boundary changes through an RFC;
- release and deprecation decisions recorded publicly;
- conflicts resolved by documented maintainer vote, not private unwritten rules.

The governance file defines maintainer nomination/removal, voting, conflicts of interest, inactivity, project transfer, and succession.

## 13. Distribution

Official releases should provide signed, checksummed artifacts for:

- macOS Apple Silicon and Intel where maintained;
- Linux x86-64 and ARM64 where maintained;
- Windows x86-64;
- container/OCI image for CI and servers;
- Rust package installation for developers.

Preferred user installations:

```bash
# package manager examples once official packages exist
brew install qbenchmed
winget install QBenchMed.QBenchMed
cargo install qbenchmed-cli
```

Do not make `curl ... | sh` the only or primary installation path. Releases include checksums, signatures, SBOM, dependency licences, upgrade notes, and database/schema migration notes.

The default installation bundles:

- CLI;
- local web server and embedded UI;
- generic scanners;
- generic adapter and policies;
- report renderer;
- sample projects;
- no mandatory Python runtime.

Quantum and framework-specific Python bridges are optional install features.

## 14. CI and developer-platform integration

Noninteractive CI command:

```bash
qbm ci audit . \
  --profile .qbench/project.yaml \
  --baseline approved.json \
  --fail-on high \
  --format sarif \
  --output qbm-results/
```

Standard outputs:

- SARIF for code-scanning systems;
- JUnit XML for CI test views;
- JSON for automation;
- Markdown for pull-request summaries;
- HTML report bundle;
- CycloneDX/SPDX-compatible dependency/SBOM export where applicable.

Exit codes:

```text
0  audit complete; policy threshold passed
1  audit complete; finding/policy threshold failed
2  audit incomplete because of configuration, parser, runner, or engine error
3  approval or governed input required in a noninteractive run
4  security/integrity boundary blocked the run
```

Provide documented examples for GitHub Actions, GitLab CI, Bitbucket Pipelines, Jenkins, and a generic container runner. The core integration remains vendor-neutral.

## 15. Documentation experience

Documentation is treated as part of the product.

Required paths:

- five-minute unknown-project quick start;
- folder-upload browser guide;
- “understanding your first report” guide;
- approval-mode guide;
- project-manifest reference;
- adapter builder tutorial;
- policy authoring guide;
- PATHEX and ONCOVA examples using non-sensitive fixtures;
- API/bulk/model connector recipes;
- CI integration recipes;
- Rust contributor architecture;
- plugin security and publishing guide;
- troubleshooting and diagnostic bundle guide;
- glossary that separates coverage, evaluability, behavior, validity, and approval.

Every error message should include a stable code and a next action. `qbm explain <code>` opens local documentation for findings and engine errors.

## 16. Examples and starter kits

Ship small synthetic projects:

- YAML rule engine with AND/OR conditions;
- Rust CLI/service project;
- Python ML pipeline with ONNX metadata;
- OpenAPI service with schema drift;
- database lineage example;
- synthetic PATHEX-like rule project;
- synthetic ONCOVA-like GUARD/CORPUS project;
- cybersecurity control-coverage project;
- education course-to-skill project.

Each example includes a healthy version, intentionally broken version, expected findings, adapter/profile, and CI workflow.

## 17. Accessibility and internationalization

- keyboard-accessible approval and findings workflows;
- screen-reader labels and semantic structure;
- severity never conveyed only by color;
- reduced-motion support;
- plain-language summaries with technical expansion;
- UI strings externalized from the start;
- locale-independent canonical artifacts;
- translated UI/report templates can be contributed independently;
- source identifiers and hashes never change through translation.

## 18. Telemetry and privacy policy

Default behavior:

```text
telemetry: OFF
crash upload: OFF
registry network access: user initiated
update check: configurable and transparent
project content upload: NEVER implicit
```

If optional telemetry is later offered:

- opt-in only;
- documented event schema;
- no paths, source content, graph content, project IDs, or sensitive metadata;
- local preview of what is sent;
- independent disable switch;
- self-hosting option for organizations where justified.

## 19. Community security process

- private vulnerability-reporting channel documented in `SECURITY.md`;
- supported-version table;
- coordinated disclosure policy;
- security advisories for core and registry plugins;
- ability to mark a plugin withdrawn or vulnerable without remotely uninstalling it;
- local warning based on signed advisory data when the user chooses to refresh;
- reproducible release and dependency evidence where practical.

## 20. Open-source acceptance criteria

The community release is ready only when:

1. A new user can install one artifact and run `qbm audit .` without Python.
2. An unknown synthetic project receives a useful generic report without a manifest.
3. The UI can import a folder/archive and explain exclusions before scanning.
4. The wizard can generate a project profile without editing YAML manually.
5. A configuration adapter can be created and tested without rebuilding Q-BenchMed.
6. A third-party WASM/process plugin can pass conformance without core modification.
7. CLI/API/report/plugin schemas are versioned and documented.
8. PATHEX and ONCOVA are examples, not hard-coded assumptions in the core.
9. The application works offline with telemetry disabled.
10. Private project data remains local unless the user explicitly exports it.
11. CI can consume SARIF/JUnit/JSON and use stable exit codes.
12. Signed releases, checksums, SBOM, licences, governance, contribution, and security documents exist.
13. Documentation includes quick starts and complete adapter examples.
14. Accessibility checks are part of UI release tests.
15. The report clearly distinguishes generic discovery, approved semantics, behavior evidence, and human validation.

## 21. Product rule

The open-source experience follows this rule:

> Give every project immediate generic value, make deeper understanding guided and shareable, keep all powerful actions explicit, and never require a central cloud to use the platform.

