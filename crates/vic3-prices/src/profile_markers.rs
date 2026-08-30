//! Feature-gated samply/`tracing` helpers for price solves.
//!
//! Compiled always so call sites stay uniform; with `profiling-markers` off the
//! tracker and solve-span guard are ZSTs (no `tracing` dependency).

use crate::result::{SolveOutcome, SolveStrategy};

/// Jac-delimited “fake Basin iteration” span across residual/cost → jacobian.
///
/// Basin’s TRF loop is not ours; we approximate one iteration as everything from
/// the first `residual`/`cost` after a jacobian (or solve start) through the end
/// of the next `jacobian` call (including faer work inside that wall time).
pub(crate) struct BasinIterTracker {
    #[cfg(feature = "profiling-markers")]
    open: std::cell::RefCell<Option<tracing::span::EnteredSpan>>,
}

impl BasinIterTracker {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(feature = "profiling-markers")]
            open: std::cell::RefCell::new(None),
        }
    }

    /// Open a fake-iter span if none is open (first residual/cost of a step).
    pub(crate) fn note_residual_or_cost(&self) {
        #[cfg(feature = "profiling-markers")]
        {
            let mut g = self.open.borrow_mut();
            if g.is_none() {
                *g = Some(tracing::info_span!("basin_iter").entered());
            }
        }
        #[cfg(not(feature = "profiling-markers"))]
        {
            let _ = self;
        }
    }

    /// Ensure a fake-iter is open before jacobian work.
    pub(crate) fn begin_jacobian(&self) {
        self.note_residual_or_cost();
    }

    /// Close the fake-iter when `jacobian` returns.
    pub(crate) fn end_jacobian(&self) {
        #[cfg(feature = "profiling-markers")]
        {
            self.open.borrow_mut().take();
        }
        #[cfg(not(feature = "profiling-markers"))]
        {
            let _ = self;
        }
    }

    /// Drop any open fake-iter at end of solve (e.g. residual-only finish).
    pub(crate) fn close(&self) {
        self.end_jacobian();
    }
}

impl Drop for BasinIterTracker {
    fn drop(&mut self) {
        self.close();
    }
}

/// Outer span for one equilibrate run; fields filled when the outcome is known.
pub(crate) struct PriceSolveSpan {
    #[cfg(feature = "profiling-markers")]
    span: tracing::Span,
}

/// RAII enter guard for [`PriceSolveSpan`] (ZST when markers are off).
pub(crate) struct PriceSolveEntered<'a> {
    #[cfg(feature = "profiling-markers")]
    _guard: tracing::span::Entered<'a>,
    #[cfg(not(feature = "profiling-markers"))]
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl PriceSolveSpan {
    pub(crate) fn new(strategy: SolveStrategy) -> Self {
        #[cfg(feature = "profiling-markers")]
        {
            Self {
                span: tracing::info_span!(
                    "price_solve",
                    strategy = ?strategy,
                    param_dim = tracing::field::Empty,
                    status = tracing::field::Empty,
                    residual = tracing::field::Empty,
                    n_residual_evals = tracing::field::Empty,
                    n_jacobian_evals = tracing::field::Empty,
                ),
            }
        }
        #[cfg(not(feature = "profiling-markers"))]
        {
            let _ = strategy;
            Self {}
        }
    }

    pub(crate) fn enter(&self) -> PriceSolveEntered<'_> {
        #[cfg(feature = "profiling-markers")]
        {
            PriceSolveEntered {
                _guard: self.span.enter(),
            }
        }
        #[cfg(not(feature = "profiling-markers"))]
        {
            let _ = self;
            PriceSolveEntered {
                _phantom: std::marker::PhantomData,
            }
        }
    }

    pub(crate) fn record(&self, outcome: &SolveOutcome) {
        #[cfg(feature = "profiling-markers")]
        {
            self.span.record("param_dim", outcome.stats.param_dim);
            self.span
                .record("status", tracing::field::debug(&outcome.status));
            self.span.record("residual", outcome.residual);
            self.span
                .record("n_residual_evals", outcome.stats.n_residual_evals);
            self.span
                .record("n_jacobian_evals", outcome.stats.n_jacobian_evals);
        }
        #[cfg(not(feature = "profiling-markers"))]
        {
            let _ = (self, outcome);
        }
    }
}
