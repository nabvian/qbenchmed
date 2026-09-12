"""Q-BenchMed: quantum-classical benchmarking for structured biomedical
input-selection problems.

The package is layered so the core knows nothing about any biomedical domain
(spec section 24):

    instances   -- domain-agnostic benchmark instance generation
    qubo        -- QUBO/Ising formulation with three coverage encodings
    classical   -- exact ILP, exhaustive, greedy, simulated annealing, tabu
    simulator   -- statevector simulator with validated gate set
    qaoa        -- QAOA ansatz, parameter optimisation, resource metrics
"""

__version__ = "0.1.0"
SCHEMA_VERSION = "qbm-result/1.0"
