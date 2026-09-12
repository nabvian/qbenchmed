//! Deterministic biomedical input optimization for Q-BenchMed.
//!
//! This crate starts only after a reviewed adapter has projected a biomedical
//! project into an explicit [`BenchmarkProfile`]. It never infers biomedical
//! meaning from source-code names, performs I/O, contacts a provider, or claims
//! quantum advantage. Its schemas and algorithms are deterministic so classical
//! and QUBO/Ising results can be reproduced and compared honestly.

pub mod coverage;
mod diff;
mod error;
pub mod ilp;
mod metrics;
mod qubo;
mod schema;
mod solve;
mod validation;

pub use coverage::{Arm, CoverageModel, InputRole, InputSet, OutcomeRule};
pub use diff::{ProfileDiff, diff_profiles};
pub use error::BenchmarkError;
pub use ilp::{IlpLimits, IlpProblem, IlpSolution, solve_max_coverage, solve_minimum_panel};
pub use metrics::{StructuralMetrics, structural_metrics};
pub use qubo::{
    DifficultyEstimate, DifficultyLevel, IsingCoupling, IsingModel, QuboCoupling, QuboMetrics,
    QuboModel, QuboVariable, QuboVariableKind, build_qubo, estimate_difficulty,
};
pub use schema::{
    BENCHMARK_PROFILE_SCHEMA_V1, BENCHMARK_PROFILE_SCHEMA_V2, BENCHMARK_PROFILE_SCHEMA_VERSION,
    BenchmarkConstraints, BenchmarkInput, BenchmarkObjective, BenchmarkOutcome, BenchmarkProfile,
    BiomedicalScope, IncidenceRelationship, ProfileProvenance, RelationshipKind,
    SUPPORTED_PROFILE_SCHEMA_VERSIONS,
};
pub use solve::{
    CoveragePoint, OptimizationRequest, OptimizationResult, SelectionScore, SolverConfig,
    SolverKind, coverage_at_k, evaluate_selection, minimum_panel, solve,
};

/// Schema version emitted by every optimization result.
pub const OPTIMIZATION_RESULT_SCHEMA_VERSION: &str = "qbm.optimization-result/v1";
/// Schema version emitted by every profile comparison.
pub const PROFILE_DIFF_SCHEMA_VERSION: &str = "qbm.profile-diff/v1";
/// Schema version emitted by QUBO models.
pub const QUBO_SCHEMA_VERSION: &str = "qbm.qubo/v1";
/// Schema version emitted by Ising models.
pub const ISING_SCHEMA_VERSION: &str = "qbm.ising/v1";
