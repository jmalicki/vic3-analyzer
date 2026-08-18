//! Dense good indices aligned with [`crate::GameDefs::goods_order`].

use std::fmt;
use std::ops::{Index, IndexMut};

use serde::{Deserialize, Serialize};

/// Index into [`crate::GameDefs::goods_order`].
///
/// Prefer this over raw `usize` so wealth levels and need slots cannot be
/// passed where a good is required.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GoodIdx(u16);

impl GoodIdx {
    /// Construct from a position when it fits the compact representation.
    #[inline]
    pub fn try_from_usize(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(Self)
    }

    /// Construct from a position in `goods_order`. Panics if `index` exceeds `u16::MAX`.
    #[inline]
    pub fn from_usize(index: usize) -> Self {
        Self::try_from_usize(index).expect("good count fits in u16")
    }

    /// Position in `goods_order`.
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

impl fmt::Debug for GoodIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GoodIdx({})", self.0)
    }
}

impl fmt::Display for GoodIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Dense per-good quantities, length equal to `goods_order.len()`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoodsVec {
    data: Vec<f64>,
}

impl GoodsVec {
    /// Zero vector of length `n`.
    pub fn zeros(n: usize) -> Self {
        Self { data: vec![0.0; n] }
    }

    /// Build from an existing dense buffer. Length must match `goods_order`.
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
    pub fn iter_indexed(&self) -> impl Iterator<Item = (GoodIdx, f64)> + '_ {
        self.data
            .iter()
            .enumerate()
            .map(|(i, &v)| (GoodIdx::from_usize(i), v))
    }

    /// Add `qty` into slot `idx` (no-op-safe for callers that already validated).
    #[inline]
    pub fn add(&mut self, idx: GoodIdx, qty: f64) {
        self.data[idx.as_usize()] += qty;
    }

    /// `self += other * scale`. Lengths must match.
    #[inline]
    pub fn add_scaled(&mut self, other: &Self, scale: f64) {
        if scale == 0.0 {
            return;
        }
        debug_assert_eq!(self.data.len(), other.data.len());
        for (dst, src) in self.data.iter_mut().zip(&other.data) {
            *dst += *src * scale;
        }
    }

    /// Overwrite from `other`. Lengths must match.
    #[inline]
    pub fn copy_from(&mut self, other: &Self) {
        debug_assert_eq!(self.data.len(), other.data.len());
        self.data.copy_from_slice(&other.data);
    }
}

impl Index<GoodIdx> for GoodsVec {
    type Output = f64;

    #[inline]
    fn index(&self, index: GoodIdx) -> &Self::Output {
        &self.data[index.as_usize()]
    }
}

impl IndexMut<GoodIdx> for GoodsVec {
    #[inline]
    fn index_mut(&mut self, index: GoodIdx) -> &mut Self::Output {
        &mut self.data[index.as_usize()]
    }
}

impl FromIterator<f64> for GoodsVec {
    fn from_iter<T: IntoIterator<Item = f64>>(iter: T) -> Self {
        Self {
            data: iter.into_iter().collect(),
        }
    }
}
