# qbm-benchmark

`qbm-benchmark` is the deterministic, pure-Rust optimization kernel for the
original Q-BenchMed scope: biomedical input/panel selection against explicitly
declared biomedical outcomes.

It deliberately has no filesystem, web, database, adapter, or quantum-provider
dependency. A caller must first project reviewed source material into the strict
`qbm.benchmark-profile/v1` schema. The crate then validates that profile and can:

- measure reachability, density, active inputs, and inert inputs;
- compare two profile versions without guessing semantic identity;
- compute weighted coverage at selected panel sizes;
- find a minimum panel for a requested coverage floor;
- run deterministic exact, greedy, simulated-annealing, and tabu baselines;
- construct and score an exact binary QUBO encoding for supported constraints;
- convert that QUBO to Ising form and report transparent size/range/difficulty
  diagnostics.

All stochastic-looking baselines use an explicit deterministic seed. The crate
does not contact a quantum provider and makes no claim of quantum advantage.

See the workspace
[biomedical workflow and profile contract](../../docs/BIOMEDICAL_WORKFLOW.md#the-qbmprofile-contract)
and the
[synthetic profile example](../../examples/profiles/nsclc-mini.qbm.profile.json).
