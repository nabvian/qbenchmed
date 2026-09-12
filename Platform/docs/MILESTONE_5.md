# Milestone 5 — Biomedical Source UI

> Historical completion record: this page describes the Milestone 5
> source-only UI. The current 14-stage biomedical workflow is documented in
> [the workspace README](../README.md) and
> [Biomedical Workflow and Architecture](BIOMEDICAL_WORKFLOW.md).

## Outcome

Q-BenchMed Platform now has a separate, local-first browser client for the
domain-blind audit core. A user can start `qbm serve`, choose a project archive
or folder, or paste a public GitHub repository root URL, and complete the
generic audit without copying run IDs or hashes between terminal commands.

PATHEX, ONCOVA, and other biomedical projects enter as ordinary source data.
No PATHEX-specific benchmark was added in this milestone. An optional
declarative biomedical adapter can interpret domain meaning in the governed
flow; it is not required for structural intake, graphing, or generic findings.

## User workflow

```text
Archive / browser folder / public GitHub repository
                     │
                     ▼
        Immutable source acquisition
                     │
                     ▼
      Check files ── manual approval
                     │
                     ▼
    Freeze evidence ─ manual approval
                     │
                     ▼
       Map structure ─ manual approval
                     │
                     ▼
    Structural audit ─ manual approval
                     │
                     ▼
      Complete + JSON audit bundle
```

Each approval is bound to the exact current SHA-256. Approval means the user
accepts that output as the next stage's input; it does not mean the project is
correct, clinically valid, secure, or issue-free.

## Rust architecture

- `qbm-web` owns the Axum HTTP service, acquisition boundaries, GitHub client,
  workflow projection, report bundle, and embedded HTML/CSS/JavaScript.
- `qbm-cli` exposes `qbm serve` and starts a Tokio runtime.
- `qbm-app` remains the only browser-facing path into core use cases.
- `qbm-store` persists immutable acquisitions and exact run-acquisition
  bindings in SQLite; acquisition manifests remain content-addressed artifacts.
- `qbm-intake` continues to parse archives without extracting them and never
  executes imported code.
- `qbm-audit` continues to build the scanner-neutral graph and generic report.

The browser contains presentation and orchestration only. It does not recreate
hashing, approval, graph, audit, or completion rules in JavaScript.

## Source acquisition contracts

### Archive

- accepted: `.zip`, `.tar`, `.tar.gz`, `.tgz`;
- streamed to an opaque managed directory;
- compressed bytes and SHA-256 are bounded and recorded;
- the original filename is display metadata only;
- archive members are parsed without extraction.

### Browser folder

- the browser sends file bytes plus `webkitRelativePath` values;
- the service rejects parent, absolute, drive-like, control-character,
  duplicate, excessive-depth, and overlong paths;
- the folder acquisition hash covers sorted path, file hash, and byte size;
- no API accepts an arbitrary server-side source path.

### Public GitHub

- accepted shape: `https://github.com/{owner}/{repository}` with an optional
  `.git` suffix or trailing slash, without
  credentials, query, fragment, port, encoded path, or extra route;
- branch/tag/commit is a separate field;
- GitHub metadata supplies stable repository identity and default branch;
- the reference resolves to one full 40-character commit SHA;
- archive request uses that commit, not a mutable branch;
- redirects are disabled by default and the single codeload destination is
  validated against owner, repository, and commit;
- the archive is streamed with a hard compressed-size bound and SHA-256;
- no token, cookie, Git subprocess, or arbitrary remote URL is used.

The downloaded archive represents one source commit, not Git history. Git LFS
content may remain pointer files depending on repository archive settings.

## Browser security boundary

- binds only to `127.0.0.1`;
- exact Host allowlist prevents DNS-rebinding-style host changes;
- every mutation requires exact same-origin `Origin` and a per-process CSRF
  header obtained from the same-origin bootstrap response;
- cross-site Fetch Metadata values are rejected;
- no permissive CORS response is emitted;
- CSP, `frame-ancestors 'none'`, `nosniff`, no-referrer, and permissions-policy
  headers are applied;
- API responses are `Cache-Control: no-store`;
- browser source ceilings are lower than trusted CLI intake ceilings.

## Durable report bundle

The download is `qbm.audit-bundle/v1`. It contains safe project metadata,
source acquisition provenance, run identity/state, exact approvals, inventory
summary/exceptions, snapshot identity/size, graph summary/diagnostics, canonical
generic findings, append-only events, and event-chain verification. It omits the
managed absolute source path and browser CSRF token.

## Verification performed

- browser ZIP upload completed all four approval gates and produced five known
  fixture findings;
- browser folder selection preserved three relative files and completed;
- live `octocat/Hello-World` import resolved and displayed an immutable commit,
  completed, and produced a downloadable report;
- page reload restored a completed run and its five findings;
- a malicious ZIP member using `../../` was rejected without extraction;
- router tests cover Host/security headers, Origin/CSRF refusal, ZIP completion,
  terminal run state, event-chain validity, report download, and path omission;
- GitHub URL and codeload redirect spoofing tests cover accepted and rejected
  forms;
- the full workspace formatting, lint, test, dependency-policy, and release
  build gates are recorded in the final verification run.

## Honest release status

This milestone is suitable for local developer testing. It is not yet a public
beta or hosted multi-user service. Background job persistence, cancellation,
private GitHub authentication, arbitrary Git providers, automatic accessibility
testing, platform installers, signed releases, SBOM publishing, retention, and
safe deletion remain future hardening work.
