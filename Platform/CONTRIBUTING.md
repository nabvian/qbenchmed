# Contributing to Q-BenchMed Platform

Thank you for helping improve Q-BenchMed. Contributions must remain within the
project's purpose: **a modular open-source quantum–classical benchmarking
framework for biomedical input optimization**.

Generic source intake and static scanning support that purpose. Q-BenchMed is
not a general-purpose code-quality product, clinical validator, diagnostic
device, regulatory assessment, or demonstration of quantum advantage.

## Before you start

- Discuss a large feature or a profile-contract change in an issue before
  implementation.
- Never add execution of uploaded source, tests, build scripts, notebooks,
  models, or neural engines. Treat every uploaded project as untrusted data.
- Do not infer biomedical truth from identifiers, filenames, or source-language
  symbols. Unsupported semantics must produce explicit diagnostics or skipped
  capabilities, not invented inputs, outcomes, or relationships.
- Preserve content-addressed provenance, exact-output approvals, deterministic
  baselines, and the distinction between classical results, formulation
  validation, export readiness, and actual provider execution.

Security-sensitive reports belong in the private channel described in
[SECURITY.md](SECURITY.md), not in a public issue.

## Development setup

Install the pinned Rust toolchain declared in `rust-toolchain.toml`. CI also
uses cargo-deny 0.20.2. From the `Platform` directory, run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check
```

Use `cargo fmt --all` to apply Rust formatting. Keep dependency changes
intentional and commit the resulting `Cargo.lock` update.

## Making a change

1. Add focused tests for behavior and boundary cases.
2. Keep parsers bounded and non-executing; include evidence and clear reasons
   for unsupported or skipped analysis.
3. Update contracts and documentation when serialized output changes.
4. Avoid provider, clinical, performance, or quantum-advantage claims that the
   implementation and reproducibility record cannot prove.
5. Run the full check set above before opening a pull request.

A pull request should explain the biomedical input-optimization use case, the
trust-boundary impact, how it was tested, and any remaining limitations. Keep
unrelated refactors in separate changes so review can follow the evidence.

By contributing, you agree that your contribution is licensed under the
repository's Apache License 2.0.
