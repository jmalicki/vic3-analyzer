//! Option and result types matching `docs/json-schema.md` (`PricesResult`).

use serde::{Deserialize, Serialize};

/// Solver iteration / residual tolerances.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SolveOpts {
    /// Residual threshold for [`SolveStatus::Converged`] (I5). Default `1e-6`.
    #[serde(default = "default_residual_eps")]
    pub residual_eps: f64,
    /// Combined successive-substitution + Basin iteration cap. Default `100`.
    #[serde(default = "default_max_iters")]
    pub max_iters: u32,
}

fn default_residual_eps() -> f64 {
    1e-6
}

fn default_max_iters() -> u32 {
    100
}

impl Default for SolveOpts {
    fn default() -> Self {
        Self {
            residual_eps: default_residual_eps(),
            max_iters: default_max_iters(),
        }
    }
}

/// Extra building levels applied before a re-solve. Employment stays frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatIfOpts {
    /// Building type id (matches [`crate::WorldBuilding::building`]).
    pub building: String,
    /// Non-negative extra levels added to matching buildings.
    pub extra_levels: u32,
}

/// Why [`crate::solve`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveStatus {
    /// [`PricesResult::residual`] is below [`SolveOpts::residual_eps`] (I5).
    Converged,
    /// Iteration budget exhausted with residual still at or above ε.
    MaxIters,
    /// Basin reported failure and successive substitution did not recover.
    Failed,
}

/// One row of the goods table in [`PricesResult`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoodPrice {
    pub id: String,
    pub base: f64,
    pub price: f64,
    pub buy: f64,
    pub sell: f64,
}

/// Price-equilibrium output. `limitations` is always the crate [`crate::LIMITATIONS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PricesResult {
    pub goods: Vec<GoodPrice>,
    /// `‖r − r_formula(orders(r))‖₂`. Always present (I5).
    pub residual: f64,
    pub status: SolveStatus,
    pub limitations: Vec<String>,
}
