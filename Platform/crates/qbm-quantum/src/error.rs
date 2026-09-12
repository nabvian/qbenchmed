//! Contract validation errors.

use qbm_domain::Sha256Digest;
use thiserror::Error;

/// Failures raised while constructing or validating quantum contracts.
#[derive(Debug, Error)]
pub enum QuantumContractError {
    /// A versioned contract used an unsupported schema identifier.
    #[error("unsupported {contract} schema {found:?}; expected {expected}")]
    UnsupportedSchema {
        /// Contract being validated.
        contract: &'static str,
        /// Supplied schema identifier.
        found: String,
        /// Accepted schema identifier.
        expected: &'static str,
    },
    /// A contract violated a structural or semantic invariant.
    #[error("invalid quantum contract: {0}")]
    InvalidContract(String),
    /// A declared content identity did not match canonical serialized content.
    #[error("{contract} identity mismatch: declared {declared}, computed {computed}")]
    IdentityMismatch {
        /// Contract whose identity was checked.
        contract: &'static str,
        /// Identity carried by the contract.
        declared: Sha256Digest,
        /// Identity recomputed from canonical content.
        computed: Sha256Digest,
    },
    /// A canonical value could not be serialized or hashed.
    #[error(transparent)]
    Canonical(#[from] qbm_canonical::CanonicalError),
    /// The underlying benchmark model or native score was invalid.
    #[error(transparent)]
    Benchmark(#[from] qbm_benchmark::BenchmarkError),
    /// Equivalent QUBO and Ising models produced different energies.
    #[error(
        "QUBO/Ising energy mismatch at generated assignment {assignment_index}: \
         QUBO={qubo_energy}, Ising={ising_energy}, error={absolute_error}, \
         allowed={allowed_error}"
    )]
    EnergyMismatch {
        /// Zero-based position in the deterministic assignment sequence.
        assignment_index: usize,
        /// QUBO energy for the assignment.
        qubo_energy: f64,
        /// Ising energy under `z = 1 - 2x`.
        ising_energy: f64,
        /// Absolute energy difference.
        absolute_error: f64,
        /// Absolute plus relative error allowed for this energy scale.
        allowed_error: f64,
    },
}

pub(crate) fn require_schema(
    contract: &'static str,
    found: &str,
    expected: &'static str,
) -> Result<(), QuantumContractError> {
    if found == expected {
        Ok(())
    } else {
        Err(QuantumContractError::UnsupportedSchema {
            contract,
            found: found.to_owned(),
            expected,
        })
    }
}
