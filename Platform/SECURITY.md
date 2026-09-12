# Security Policy

Q-BenchMed analyses untrusted biomedical project artifacts. Its current
browser service is a local, single-user tool bound to loopback; it is not an
authenticated multi-user service and must not be exposed directly to an
untrusted network.

Before tagged releases exist, security fixes target the current development
branch. Once supported releases are published, this file should list their
support windows explicitly.

## Report a vulnerability privately

Use GitHub's **Security → Report a vulnerability** feature for this repository
when it is available. If private vulnerability reporting is unavailable,
contact the repository maintainers through a private contact method published
by the repository owner. A public issue may request a private contact channel,
but must not contain exploit details.

Include the affected revision, impact, reproduction steps, and the smallest
safe proof of concept. Do not send credentials, protected health information,
patient data, proprietary biomedical datasets, or an unsafe live exploit.
Maintainers will coordinate disclosure after the report can be reproduced and
a remediation is available; response timing depends on maintainer availability.

## Security-relevant issues

Examples include:

- execution of uploaded code or escape from a parser or adapter boundary;
- archive traversal, symlink, duplicate-path, decompression, or resource-limit
  bypasses;
- server-side request forgery or GitHub source-validation bypasses;
- access outside the configured data directory or disclosure of local files;
- approval, artifact-hash, provenance, or reproducibility-chain bypasses;
- browser injection, origin/host-check bypasses, or unsafe report rendering;
- credential leakage or a vulnerable dependency with an exploitable path.

Ordinary parser gaps, unsupported languages, optimization quality questions,
biomedical-content disagreements, and claims about clinical validity or quantum
advantage are generally not security vulnerabilities unless they cross a trust
boundary or permit falsification of recorded evidence.

## Safe deployment and data handling

- Keep the service on loopback and behind the supplied host, origin, CSRF, and
  content-security controls.
- Analyse only data you are authorized to use. Uploaded paths, samples, and
  report artifacts may be sensitive even though source code is never executed.
- Do not place secrets, provider credentials, or patient data in a project
  upload, `qbm.profile`, log, issue, or exported audit bundle.
- Treat QUBO/Ising output as a validated formulation export until a separately
  reviewed executor records a provider job. It is not evidence of quantum
  execution or advantage.
- Treat every result as benchmarking evidence, not clinical advice, diagnosis,
  regulatory approval, or proof of biomedical truth.
