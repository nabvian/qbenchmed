//! Error types for benchmark validation and bounded optimization.

use thiserror::Error;

/// Failure raised by strict profile validation or a bounded solver.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum BenchmarkError {
    /// The supplied profile does not satisfy the versioned schema invariants.
    #[error("invalid benchmark profile: {0}")]
    InvalidProfile(String),
    /// An optimization request is contradictory or outside the profile bounds.
    #[error("invalid optimization request: {0}")]
    InvalidRequest(String),
    /// No selection can satisfy the requested constraints.
    #[error("the optimization problem is infeasible: {0}")]
    Infeasible(String),
    /// Exact enumeration would exceed the caller's explicit safety ceiling.
    #[error(
        "exact optimization requires {required_states} states, above the limit of {maximum_states}"
    )]
    ExactLimit {
        /// Number of bit-mask states required by the input count.
        required_states: u64,
        /// Configured maximum number of enumerated states.
        maximum_states: u64,
    },
    /// The exact QUBO encoding cannot represent a declared constraint honestly.
    #[error("the exact QUBO encoder does not support {0}")]
    UnsupportedQuboConstraint(String),
    /// A supplied QUBO/Ising assignment has the wrong length or an invalid value.
    #[error("invalid binary/spin assignment: {0}")]
    InvalidAssignment(String),
    /// Arithmetic produced a non-finite coefficient and was rejected.
    #[error("non-finite numeric result while computing {0}")]
    NonFinite(&'static str),
}
