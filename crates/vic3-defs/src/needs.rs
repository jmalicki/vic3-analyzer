//! Dense need indices aligned with [`crate::GameDefs::needs_order`].
//!
//! Needs are consumption categories (`popneed_heating`, …), not market goods.
//! Buy packages budget by need; substitution maps each need onto goods.

use std::fmt;
use std::ops::{Index, IndexMut};

use serde::{Deserialize, Serialize};

/// Index into [`crate::GameDefs::needs_order`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NeedIdx(u16);

impl NeedIdx {
    /// Construct from a position when it fits the compact representation.
    #[inline]
    pub fn try_from_usize(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(Self)
    }

    /// Construct from a position in `needs_order`. Panics if `index` exceeds `u16::MAX`.
    #[inline]
    pub fn from_usize(index: usize) -> Self {
        Self::try_from_usize(index).expect("need count fits in u16")
    }

    /// Position in `needs_order`.
    #[inline]
    pub fn as_usize(self) -> usize {
        usize::from(self.0)
    }

    /// Raw discriminant for tests and sparse tables.
    #[inline]
    pub fn raw(self) -> u16 {
        self.0
    }
}

impl fmt::Debug for NeedIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NeedIdx({})", self.0)
    }
}

impl fmt::Display for NeedIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Dense per-need quantities, length equal to `needs_order.len()`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NeedsVec {
    data: Vec<f64>,
}

impl NeedsVec {
    /// Zero vector of length `n`.
    pub fn zeros(n: usize) -> Self {
        Self { data: vec![0.0; n] }
    }

    /// Build from an existing dense buffer. Length must match `needs_order`.
    pub fn from_vec(data: Vec<f64>) -> Self {
        Self { data }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[inline]
    pub fn as_slice(&self) -> &[f64] {
        &self.data
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [f64] {
        &mut self.data
    }

    /// Copy into a vector of exactly `n` slots, zero-filling missing slots.
    pub fn aligned(&self, n: usize) -> Self {
        let mut out = Self::zeros(n);
        let copied = n.min(self.len());
        out.data[..copied].copy_from_slice(&self.data[..copied]);
        out
    }

    /// `(index, value)` pairs in order.
    pub fn iter_indexed(&self) -> impl Iterator<Item = (NeedIdx, f64)> + '_ {
        self.data
            .iter()
            .enumerate()
            .map(|(i, &v)| (NeedIdx::from_usize(i), v))
    }

    /// Linear blend `a * (1 - t) + b * t`.
    pub fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        let n = a.len().max(b.len());
        let mut out = Self::zeros(n);
        for i in 0..n {
            let lo = a.data.get(i).copied().unwrap_or(0.0);
            let hi = b.data.get(i).copied().unwrap_or(0.0);
            out.data[i] = lo * (1.0 - t) + hi * t;
        }
        out
    }

    /// Add `qty` into slot `idx`.
    #[inline]
    pub fn add(&mut self, idx: NeedIdx, qty: f64) {
        self.data[idx.as_usize()] += qty;
    }
}

impl Index<NeedIdx> for NeedsVec {
    type Output = f64;

    #[inline]
    fn index(&self, index: NeedIdx) -> &Self::Output {
        &self.data[index.as_usize()]
    }
}

impl IndexMut<NeedIdx> for NeedsVec {
    #[inline]
    fn index_mut(&mut self, index: NeedIdx) -> &mut Self::Output {
        &mut self.data[index.as_usize()]
    }
}

impl FromIterator<f64> for NeedsVec {
    fn from_iter<T: IntoIterator<Item = f64>>(iter: T) -> Self {
        Self {
            data: iter.into_iter().collect(),
        }
    }
}
