//! Block-arrowhead Jacobian and structured Gram for Basin [`Trf`](basin::Trf).
//!
//! Vic3 joint price equilibrium stores the NLS Jacobian as an [`ArrowheadMat`]:
//! a market hub (`G` rows) coupled to `S` independent state blocks (each `G×G` dense).
//! Storage is **`O(S·G²)`** instead of materializing the full `n×n` Gram with
//! `n = G(1+S)`.
//!
//! # Partition
//!
//! Unknowns `x = [r (G) | σ₀ (G) | … | σ_{S−1} (G)]`, dimension `n = G(1+S)`.
//!
//! ```text
//!        r    σ₀   σ₁  …  σ_{S−1}
//! mkt  [ M_r  M_0  M_1 …  M_{S−1} ]   ← G market rows
//! σ₀   [ A_0  C_0   0   …    0    ]   ← G rows per state
//! σ₁   [ B_1   0   C_1  …    0    ]
//! …
//! ```
//!
//! Each block is dense `G×G` (wealth / Laspeyres couples goods within a state).
//!
//! # Gram and [`LinearSolveSpd`](basin::LinearSolveSpd)
//!
//! Basin [`GramMatrix::gram`] returns `Self`, so one type holds both the Jacobian
//! and its normal-equations operator. After [`ArrowheadMat::gram`], the matrix is
//! in SPD mode:
//!
//! ```text
//! JᵀJ = A_local + MᵀM
//! ```
//!
//! where `A_local` is the arrowhead from **state** rows only, and `M` is the `G×n`
//! block from **market** rows. [`AddDiagonalVectorInPlace`](basin::AddDiagonalVectorInPlace)
//! adds Basin TRF damping (`c + μ·d²`) to a per-coordinate diagonal without
//! materializing a full `n×n` Gram.
//!
//! [`LinearSolveSpd::solve_spd`] factors `A_local + diag(d)` with a hub Schur
//! (`O(S·G³)`), then applies Woodbury for the rank‑`G` market term `MᵀM` via a
//! single `G×G` correction.
//!
//! # Example
//!
//! ```rust
//! use basin::{AddDiagonalVectorInPlace, GramMatrix, LinearSolveSpd, MatVec};
//! use basin_arrowhead::ArrowheadMat;
//! use faer::Col;
//!
//! let g = 2;
//! let s = 1;
//! let mut jac = ArrowheadMat::zeros(g, s);
//! jac.set_entry(0, 0, 1.0);
//! jac.set_entry(g, g, 2.0); // state-0, σ₀ diagonal
//!
//! let x = Col::from_fn(g * (1 + s), |i| 0.1 * (i as f64 + 1.0));
//! let _y = jac.matvec(&x);
//!
//! let mut gram = jac.gram();
//! let damp = Col::from_fn(gram.ncols(), |_| 1e-4);
//! gram.add_diagonal_vector_in_place(&damp);
//! let rhs = Col::from_fn(gram.nrows(), |i| (i as f64).sin());
//! let step = gram.solve_spd(&rhs).expect("damped Gram is SPD");
//! assert_eq!(step.nrows(), gram.nrows());
//! ```

mod matrix;

pub use matrix::ArrowheadMat;
