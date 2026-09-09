use basin::{
    AddDiagonalVectorInPlace, GramMatrix, LinearSolveError, LinearSolveSpd, MatTransposeVec,
    MatVec, MaxDiagonal,
};
use faer::linalg::matmul::matmul;
use faer::linalg::solvers::{Llt, Solve};
use faer::{Accum, Col, Mat, Par, Side};

/// Block-arrowhead Jacobian / Gram for joint price equilibrium.
///
/// # Coordinate layout
///
/// Rows and columns index the joint unknown vector
/// `x = [r₀,…,r_{G−1}, σ_{0,0},…,σ_{0,G−1}, σ_{1,0},…]`.
///
/// | Row / column range | Block | Meaning |
/// | --- | --- | --- |
/// | `0..G` | market | Worldwide relative prices `r` |
/// | `G+s·G .. G+(s+1)·G` | state `s` | Pure-state absolute prices `σ_s` |
///
/// In **Jacobian mode** (before [`gram`](basin::GramMatrix::gram)), use
/// [`set_entry`](Self::set_entry) or the block setters. In **SPD mode** (after
/// `gram()`), the value implements [`LinearSolveSpd`](basin::LinearSolveSpd) on
/// `JᵀJ` plus any damping added via
/// [`add_diagonal_vector_in_place`](basin::AddDiagonalVectorInPlace).
///
/// # Examples
///
/// See the [crate-level example](crate#example) for a minimal matvec / solve flow.
#[derive(Clone, Debug)]
pub struct ArrowheadMat {
    g: usize,
    s: usize,
    mode: Mode,
    /// Market rows × `r` (`G×G`).
    market_r: Mat<f64>,
    /// Market rows × `σ_s` for each state (`S` blocks of `G×G`).
    market_sigma: Vec<Mat<f64>>,
    /// State rows × `r` (`S` blocks of `G×G`); Jacobian mode only.
    state_r: Vec<Mat<f64>>,
    /// State rows × own `σ_s` (`S` blocks of `G×G`); Jacobian mode only.
    state_sigma: Vec<Mat<f64>>,
    /// Hub block `Σ_s A_s^T A_s` (`G×G`); SPD mode only.
    hub: Mat<f64>,
    /// Hub–state coupling `A_s^T C_s` (`S` blocks of `G×G`); SPD mode only.
    hub_sigma: Vec<Mat<f64>>,
    /// State diagonal `C_s^T C_s` (`S` blocks of `G×G`); SPD mode only.
    state_gram: Vec<Mat<f64>>,
    /// Extra diagonal added by Basin damping (`c + μ·d²`).
    extra_diag: Col<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Jacobian,
    Spd,
}

impl ArrowheadMat {
    /// Number of priced goods `G`.
    pub fn n_goods(&self) -> usize {
        self.g
    }

    /// Number of states `S`.
    pub fn n_states(&self) -> usize {
        self.s
    }

    /// Parameter dimension `n = G(1+S)`.
    pub fn ncols(&self) -> usize {
        self.n()
    }

    /// Same as [`Self::ncols`] (square Jacobian / Gram).
    pub fn nrows(&self) -> usize {
        self.n()
    }

    fn n(&self) -> usize {
        self.g * (1 + self.s)
    }

    /// Zero Jacobian in **Jacobian mode** with shape `(G, S)`.
    ///
    /// All blocks start at zero; fill via [`set_entry`](Self::set_entry) or the
    /// `set_*` block helpers before calling [`gram`](basin::GramMatrix::gram).
    pub fn zeros(g: usize, s: usize) -> Self {
        let blocks = |n| (0..n).map(|_| Mat::<f64>::zeros(g, g)).collect();
        Self {
            g,
            s,
            mode: Mode::Jacobian,
            market_r: Mat::zeros(g, g),
            market_sigma: blocks(s),
            state_r: blocks(s),
            state_sigma: blocks(s),
            hub: Mat::zeros(g, g),
            hub_sigma: Vec::new(),
            state_gram: Vec::new(),
            extra_diag: Col::zeros(g * (1 + s)),
        }
    }

    /// Whether this value is in structured SPD (post-`gram`) mode.
    pub fn is_spd(&self) -> bool {
        self.mode == Mode::Spd
    }

    /// Approximate number of stored `f64` values (for allocation regression tests).
    pub fn structured_storage_len(&self) -> usize {
        let block = self.g * self.g;
        match self.mode {
            Mode::Jacobian => block * (1 + 3 * self.s),
            Mode::Spd => block * (1 + 2 * self.s) + self.extra_diag.nrows(),
        }
    }

    /// Write `market_r[(row, col)]` (Jacobian mode only).
    pub fn set_market_r(&mut self, row: usize, col: usize, value: f64) {
        assert_eq!(self.mode, Mode::Jacobian);
        self.market_r[(row, col)] = value;
    }

    pub fn set_market_sigma(&mut self, state: usize, row: usize, col: usize, value: f64) {
        assert_eq!(self.mode, Mode::Jacobian);
        self.market_sigma[state][(row, col)] = value;
    }

    pub fn set_state_r(&mut self, state: usize, row: usize, col: usize, value: f64) {
        assert_eq!(self.mode, Mode::Jacobian);
        self.state_r[state][(row, col)] = value;
    }

    pub fn set_state_sigma(&mut self, state: usize, row: usize, col: usize, value: f64) {
        assert_eq!(self.mode, Mode::Jacobian);
        self.state_sigma[state][(row, col)] = value;
    }

    /// Entry `(row, col)` — for tests, finite-difference checks, and
    /// [`to_dense`](Self::to_dense). In SPD mode, returns the market blocks of
    /// `J` (not the assembled Gram); prefer trait ops for matvec / solve.
    pub fn get(&self, row: usize, col: usize) -> f64 {
        let g = self.g;
        if row < g {
            if col < g {
                return self.market_r[(row, col)];
            }
            let state = (col - g) / g;
            let c = col - g - state * g;
            return self.market_sigma[state][(row, c)];
        }
        let state = (row - g) / g;
        let r = row - g - state * g;
        if col < g {
            if self.mode == Mode::Jacobian {
                return self.state_r[state][(r, col)];
            }
            return 0.0;
        }
        let col_state = (col - g) / g;
        if col_state != state {
            return 0.0;
        }
        let c = col - g - state * g;
        if self.mode == Mode::Jacobian {
            self.state_sigma[state][(r, c)]
        } else {
            0.0
        }
    }

    /// Dense `n×n` materialization of **`J`** (Jacobian mode) — **tests / oracles
    /// only**; cost is `O(n²)`.
    pub fn to_dense(&self) -> Mat<f64> {
        let n = self.n();
        let mut out = Mat::<f64>::zeros(n, n);
        for i in 0..n {
            for j in 0..n {
                out[(i, j)] = self.get(i, j);
            }
        }
        out
    }

    fn m_times_vec(&self, x: &Col<f64>) -> Col<f64> {
        let g = self.g;
        let mut y = Col::<f64>::zeros(g);
        for i in 0..g {
            let mut sum = 0.0;
            for j in 0..g {
                sum += self.market_r[(i, j)] * x[j];
            }
            for (s, block) in self.market_sigma.iter().enumerate() {
                let base = g + s * g;
                for j in 0..g {
                    sum += block[(i, j)] * x[base + j];
                }
            }
            y[i] = sum;
        }
        y
    }

    fn gram_diagonal_entry(&self, idx: usize) -> f64 {
        let g = self.g;
        debug_assert!(idx < self.n());
        if self.mode == Mode::Jacobian {
            let mut val = 0.0;
            if idx < g {
                for i in 0..g {
                    val += self.market_r[(i, idx)].powi(2);
                }
                for block in &self.state_r {
                    for i in 0..g {
                        val += block[(i, idx)].powi(2);
                    }
                }
            } else {
                let state = (idx - g) / g;
                let c = idx - g - state * g;
                for i in 0..g {
                    val += self.market_sigma[state][(i, c)].powi(2);
                }
                for i in 0..g {
                    val += self.state_sigma[state][(i, c)].powi(2);
                }
            }
            val
        } else {
            let mut val = 0.0;
            if idx < g {
                val += self.hub[(idx, idx)];
                for i in 0..g {
                    val += self.market_r[(i, idx)].powi(2);
                }
                for block in &self.market_sigma {
                    for i in 0..g {
                        val += block[(i, idx)].powi(2);
                    }
                }
            } else {
                let state = (idx - g) / g;
                let c = idx - g - state * g;
                val += self.state_gram[state][(c, c)];
                for i in 0..g {
                    val += self.market_sigma[state][(i, c)].powi(2);
                }
            }
            val + self.extra_diag[idx]
        }
    }

    /// Apply the hub Schur complement `S = (H+D_hub) − Σ K B⁻¹ Kᵀ` to `v` without
    /// forming `K B⁻¹ Kᵀ` explicitly (avoids catastrophic cancellation).
    fn schur_apply(&self, v: &Col<f64>, state_factors: &[Llt<f64>]) -> Col<f64> {
        let g = self.g;
        let mut out = Col::<f64>::zeros(g);
        for i in 0..g {
            let mut sum = self.extra_diag[i] * v[i];
            for j in 0..g {
                sum += self.hub[(i, j)] * v[j];
            }
            out[i] = sum;
        }
        for (st, llt) in state_factors.iter().enumerate() {
            let mut kt_v = Col::<f64>::zeros(g);
            for j in 0..g {
                let mut s = 0.0;
                for i in 0..g {
                    s += self.hub_sigma[st][(i, j)] * v[i];
                }
                kt_v[j] = s;
            }
            llt.solve_in_place(kt_v.as_mut());
            let mut kw = Col::<f64>::zeros(g);
            matvec_square(&self.hub_sigma[st], &kt_v, &mut kw);
            for i in 0..g {
                out[i] -= kw[i];
            }
        }
        out
    }

    fn build_schur_hub(&self, state_factors: &[Llt<f64>]) -> Mat<f64> {
        let g = self.g;
        let mut schur = Mat::<f64>::zeros(g, g);
        for j in 0..g {
            let mut e = Col::<f64>::zeros(g);
            e[j] = 1.0;
            let col = self.schur_apply(&e, state_factors);
            for i in 0..g {
                schur[(i, j)] = col[i];
            }
        }
        schur
    }

    fn llt_with_jitter(a: &Mat<f64>) -> Result<Llt<f64>, LinearSolveError> {
        match Llt::new(a.as_ref(), Side::Lower) {
            Ok(f) => Ok(f),
            Err(_) => {
                let g = a.nrows();
                let scale = (0..g).map(|i| a[(i, i)].abs()).fold(1.0_f64, f64::max);
                let mut reg = a.clone();
                for i in 0..g {
                    reg[(i, i)] += 1e-10 * scale;
                }
                Llt::new(reg.as_ref(), Side::Lower)
                    .map_err(|_| LinearSolveError::NotPositiveDefinite)
            }
        }
    }

    fn factor_state_blocks(&self) -> Result<Vec<Llt<f64>>, LinearSolveError> {
        let g = self.g;
        let mut state_factors = Vec::with_capacity(self.s);
        for (st, bs) in self.state_gram.iter().enumerate() {
            let mut block = bs.clone();
            let base = g + st * g;
            for i in 0..g {
                block[(i, i)] += self.extra_diag[base + i];
            }
            let llt = Llt::new(block.as_ref(), Side::Lower)
                .map_err(|_| LinearSolveError::NotPositiveDefinite)?;
            state_factors.push(llt);
        }
        Ok(state_factors)
    }

    fn factor_arrowhead(&self) -> Result<(Vec<Llt<f64>>, Llt<f64>), LinearSolveError> {
        let state_factors = self.factor_state_blocks()?;
        let schur = self.build_schur_hub(&state_factors);
        let hub_llt = Self::llt_with_jitter(&schur)?;
        Ok((state_factors, hub_llt))
    }

    fn solve_arrowhead(
        &self,
        y: &Col<f64>,
        state_factors: &[Llt<f64>],
        hub_llt: &Llt<f64>,
    ) -> Col<f64> {
        let g = self.g;
        let n = self.n();
        let mut out = Col::<f64>::zeros(n);

        let mut w = vec![Col::<f64>::zeros(g); self.s];
        for (st, llt) in state_factors.iter().enumerate() {
            let base = g + st * g;
            w[st] = col_slice(y, base, g);
            llt.solve_in_place(w[st].as_mut());
        }

        let mut y_hub = col_slice(y, 0, g);
        for (st, ws) in w.iter().enumerate() {
            let mut kw = Col::<f64>::zeros(g);
            matvec_square(&self.hub_sigma[st], ws, &mut kw);
            for i in 0..g {
                y_hub[i] -= kw[i];
            }
        }
        let mut x_r = y_hub;
        hub_llt.solve_in_place(x_r.as_mut());
        for i in 0..g {
            out[i] = x_r[i];
        }

        for (st, llt) in state_factors.iter().enumerate() {
            let base = g + st * g;
            let mut inv_kt_x = Col::from_fn(g, |i| {
                let mut sum = 0.0;
                for j in 0..g {
                    sum += self.hub_sigma[st][(j, i)] * x_r[j];
                }
                sum
            });
            llt.solve_in_place(inv_kt_x.as_mut());
            for i in 0..g {
                out[base + i] = w[st][i] - inv_kt_x[i];
            }
        }
        out
    }

    fn market_row_as_col(&self, row: usize) -> Col<f64> {
        let g = self.g;
        let n = self.n();
        Col::from_fn(n, |j| {
            if j < g {
                self.market_r[(row, j)]
            } else {
                let st = (j - g) / g;
                let c = j - g - st * g;
                self.market_sigma[st][(row, c)]
            }
        })
    }

    /// Set one Jacobian entry `(row, col)` following the [coordinate layout](Self).
    ///
    /// Cross-state `σ_s`–`σ_t` blocks stay zero (no-op). Values with `|v| ≤ 1e-12`
    /// are skipped so the sparsity pattern matches the legacy triplet assembler.
    pub fn set_entry(&mut self, row: usize, col: usize, value: f64) {
        if value.abs() <= 1e-12 {
            return;
        }
        let g = self.g;
        assert_eq!(self.mode, Mode::Jacobian);
        if row < g {
            if col < g {
                self.market_r[(row, col)] = value;
            } else {
                let state = (col - g) / g;
                let c = col - g - state * g;
                self.market_sigma[state][(row, c)] = value;
            }
        } else {
            let state = (row - g) / g;
            let r = row - g - state * g;
            if col < g {
                self.state_r[state][(r, col)] = value;
            } else {
                let col_state = (col - g) / g;
                if col_state == state {
                    let c = col - g - state * g;
                    self.state_sigma[state][(r, c)] = value;
                }
            }
        }
    }

    #[cfg(test)]
    fn a_local_dense(&self) -> Mat<f64> {
        assert_eq!(self.mode, Mode::Spd);
        let g = self.g;
        let n = self.n();
        let mut a = Mat::<f64>::zeros(n, n);
        for i in 0..g {
            for j in 0..g {
                a[(i, j)] = self.hub[(i, j)];
            }
        }
        for (st, ks) in self.hub_sigma.iter().enumerate() {
            let base = g + st * g;
            for i in 0..g {
                for j in 0..g {
                    a[(i, base + j)] = ks[(i, j)];
                    a[(base + j, i)] = ks[(i, j)];
                }
            }
        }
        for (st, bs) in self.state_gram.iter().enumerate() {
            let base = g + st * g;
            for i in 0..g {
                for j in 0..g {
                    a[(base + i, base + j)] = bs[(i, j)];
                }
            }
        }
        a
    }
}

impl MatVec<Col<f64>> for ArrowheadMat {
    fn matvec(&self, x: &Col<f64>) -> Col<f64> {
        assert_eq!(self.mode, Mode::Jacobian);
        assert_eq!(x.nrows(), self.n());
        let g = self.g;
        let n = self.n();
        let mut y = Col::<f64>::zeros(n);
        let m_y = self.m_times_vec(x);
        for i in 0..g {
            y[i] = m_y[i];
        }
        for (st, (ar, ac)) in self.state_r.iter().zip(self.state_sigma.iter()).enumerate() {
            let base = g + st * g;
            for i in 0..g {
                let mut sum = 0.0;
                for j in 0..g {
                    sum += ar[(i, j)] * x[j] + ac[(i, j)] * x[base + j];
                }
                y[base + i] = sum;
            }
        }
        y
    }
}

impl MatTransposeVec<Col<f64>> for ArrowheadMat {
    fn mat_transpose_vec(&self, x: &Col<f64>) -> Col<f64> {
        assert_eq!(self.mode, Mode::Jacobian);
        assert_eq!(x.nrows(), self.n());
        let g = self.g;
        let mut y = Col::<f64>::zeros(self.n());
        for j in 0..g {
            let mut sum = 0.0;
            for i in 0..g {
                sum += self.market_r[(i, j)] * x[i];
            }
            y[j] = sum;
        }
        for (st, (ar, ac)) in self.state_r.iter().zip(self.state_sigma.iter()).enumerate() {
            let base = g + st * g;
            let sigma = &self.market_sigma[st];
            for j in 0..g {
                let mut hub = 0.0;
                let mut block = 0.0;
                for i in 0..g {
                    hub += ar[(i, j)] * x[base + i];
                    block += sigma[(i, j)] * x[i] + ac[(i, j)] * x[base + i];
                }
                y[j] += hub;
                y[base + j] = block;
            }
        }
        y
    }
}

impl GramMatrix for ArrowheadMat {
    fn gram(&self) -> Self {
        if self.mode == Mode::Spd {
            return self.clone();
        }
        let g = self.g;
        let mut hub = Mat::<f64>::zeros(g, g);
        let mut hub_sigma = Vec::with_capacity(self.s);
        let mut state_gram = Vec::with_capacity(self.s);
        for (ar, ac) in self.state_r.iter().zip(self.state_sigma.iter()) {
            matmul(
                hub.as_mut(),
                Accum::Add,
                ar.transpose(),
                ar.as_ref(),
                1.0,
                Par::Seq,
            );
            let mut ks = Mat::<f64>::zeros(g, g);
            matmul(
                ks.as_mut(),
                Accum::Replace,
                ar.transpose(),
                ac.as_ref(),
                1.0,
                Par::Seq,
            );
            hub_sigma.push(ks);
            let mut sg = Mat::<f64>::zeros(g, g);
            matmul(
                sg.as_mut(),
                Accum::Replace,
                ac.transpose(),
                ac.as_ref(),
                1.0,
                Par::Seq,
            );
            state_gram.push(sg);
        }
        Self {
            g: self.g,
            s: self.s,
            mode: Mode::Spd,
            market_r: self.market_r.clone(),
            market_sigma: self.market_sigma.clone(),
            state_r: Vec::new(),
            state_sigma: Vec::new(),
            hub,
            hub_sigma,
            state_gram,
            extra_diag: Col::zeros(self.n()),
        }
    }
}

impl MaxDiagonal for ArrowheadMat {
    fn max_diagonal(&self) -> f64 {
        (0..self.n())
            .map(|i| self.gram_diagonal_entry(i))
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

impl AddDiagonalVectorInPlace<Col<f64>> for ArrowheadMat {
    fn add_diagonal_vector_in_place(&mut self, diag: &Col<f64>) {
        assert_eq!(self.mode, Mode::Spd);
        assert_eq!(diag.nrows(), self.n());
        for i in 0..self.n() {
            self.extra_diag[i] += diag[i];
        }
    }
}

impl LinearSolveSpd<Col<f64>> for ArrowheadMat {
    fn solve_spd(&self, b: &Col<f64>) -> Result<Col<f64>, LinearSolveError> {
        assert_eq!(self.mode, Mode::Spd);
        assert_eq!(b.nrows(), self.n());

        let (state_factors, hub_llt) = self.factor_arrowhead()?;
        let t = self.solve_arrowhead(b, &state_factors, &hub_llt);

        if self.s == 0 && self.g == 0 {
            return Ok(t);
        }

        let g = self.g;
        let n = self.n();
        let mut y_cols = Mat::<f64>::zeros(n, g);
        for j in 0..g {
            let m_col = self.market_row_as_col(j);
            let col = self.solve_arrowhead(&m_col, &state_factors, &hub_llt);
            for i in 0..n {
                y_cols[(i, j)] = col[i];
            }
        }

        let mut m_mat = Mat::<f64>::zeros(g, n);
        for i in 0..g {
            let row = self.market_row_as_col(i);
            for k in 0..n {
                m_mat[(i, k)] = row[k];
            }
        }
        let mut my = Mat::<f64>::zeros(g, g);
        matmul(
            my.as_mut(),
            Accum::Replace,
            m_mat.as_ref(),
            y_cols.as_ref(),
            1.0,
            Par::Seq,
        );
        for i in 0..g {
            my[(i, i)] += 1.0;
        }

        let w_llt = Self::llt_with_jitter(&my)?;

        let u = self.m_times_vec(&t);
        let mut z = u;
        w_llt.solve_in_place(z.as_mut());

        let mut yz = Col::<f64>::zeros(n);
        for j in 0..g {
            for i in 0..n {
                yz[i] += y_cols[(i, j)] * z[j];
            }
        }

        let mut x = t;
        for i in 0..n {
            x[i] -= yz[i];
        }
        Ok(x)
    }
}

fn col_slice(v: &Col<f64>, start: usize, len: usize) -> Col<f64> {
    Col::from_fn(len, |i| v[start + i])
}

fn matvec_square(a: &Mat<f64>, x: &Col<f64>, out: &mut Col<f64>) {
    let g = x.nrows();
    for i in 0..g {
        let mut sum = 0.0;
        for j in 0..g {
            sum += a[(i, j)] * x[j];
        }
        out[i] = sum;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use faer::sparse::{SparseColMat, Triplet};

    fn damp(n: usize) -> Col<f64> {
        Col::from_fn(n, |_| 1e-4)
    }

    fn random_jacobian(g: usize, s: usize, seed: u64) -> ArrowheadMat {
        fn fill(m: &mut Mat<f64>, g: usize, seed: u64, salt: u64) {
            for i in 0..g {
                for j in 0..g {
                    let t = seed
                        .wrapping_mul(1_000)
                        .wrapping_add(salt)
                        .wrapping_add((i * g + j) as u64) as f64;
                    m[(i, j)] = (t * 0.001).sin() * 0.5;
                }
            }
        }
        let mut jac = ArrowheadMat::zeros(g, s);
        fill(&mut jac.market_r, g, seed, 1);
        for i in 0..g {
            jac.market_r[(i, i)] += 2.0;
        }
        for (idx, block) in jac.market_sigma.iter_mut().enumerate() {
            fill(block, g, seed, 10 + idx as u64);
        }
        for (idx, block) in jac.state_r.iter_mut().enumerate() {
            fill(block, g, seed, 100 + idx as u64);
        }
        for (idx, block) in jac.state_sigma.iter_mut().enumerate() {
            fill(block, g, seed, 1_000 + idx as u64);
            for i in 0..g {
                block[(i, i)] += g as f64 + 3.0;
            }
        }
        jac
    }

    fn dense_gram(jac: &ArrowheadMat) -> Mat<f64> {
        let j = jac.to_dense();
        let n = jac.n();
        let mut g = Mat::<f64>::zeros(n, n);
        matmul(
            g.as_mut(),
            Accum::Replace,
            j.transpose(),
            j.as_ref(),
            1.0,
            Par::Seq,
        );
        g
    }

    fn dense_solve_spd(a: &Mat<f64>, b: &Col<f64>) -> Col<f64> {
        let llt = Llt::new(a.as_ref(), Side::Lower).expect("spd");
        let mut x = b.clone();
        llt.solve_in_place(x.as_mut());
        x
    }

    #[test]
    fn max_diagonal_matches_dense_gram() {
        let jac = random_jacobian(4, 3, 42);
        let mut gram = jac.gram();
        let damp = damp(gram.n());
        gram.add_diagonal_vector_in_place(&damp);
        let structured = gram.max_diagonal();

        let mut dense = dense_gram(&jac);
        for i in 0..gram.n() {
            dense[(i, i)] += damp[i];
        }
        let mut dense_max = f64::NEG_INFINITY;
        for i in 0..gram.n() {
            dense_max = dense_max.max(dense[(i, i)]);
        }
        assert!(
            (structured - dense_max).abs() < 1e-10,
            "max_diagonal: structured={structured} dense={dense_max}",
        );
    }

    #[test]
    fn matvec_matches_dense() {
        let jac = random_jacobian(3, 2, 1);
        let j = jac.to_dense();
        let x = Col::from_fn(jac.n(), |i| (i as f64 + 1.0) * 0.1);
        let y = jac.matvec(&x);
        let mut y_dense = Col::<f64>::zeros(jac.n());
        matmul(
            y_dense.as_mut().as_mat_mut(),
            Accum::Replace,
            j.as_ref(),
            x.as_mat(),
            1.0,
            Par::Seq,
        );
        for i in 0..jac.n() {
            assert!((y[i] - y_dense[i]).abs() < 1e-12, "row {i}");
        }
    }

    #[test]
    fn mat_transpose_vec_matches_dense() {
        // Several state counts: the block walk indexes each state's slice off
        // `g + st * g`, so a base-offset slip only shows up past the first block.
        for (g_dim, states) in [(3, 1), (3, 2), (4, 5)] {
            let jac = random_jacobian(g_dim, states, 2);
            let j = jac.to_dense();
            let r = Col::from_fn(jac.n(), |i| (i as f64 + 1.0) * 0.07);
            let g = jac.mat_transpose_vec(&r);
            let mut g_dense = Col::<f64>::zeros(jac.n());
            matmul(
                g_dense.as_mut().as_mat_mut(),
                Accum::Replace,
                j.transpose(),
                r.as_mat(),
                1.0,
                Par::Seq,
            );
            for i in 0..jac.n() {
                assert!(
                    (g[i] - g_dense[i]).abs() < 1e-12,
                    "g={g_dim} s={states} col {i}"
                );
            }
        }
    }

    fn a_local_dense(gram: &ArrowheadMat) -> Mat<f64> {
        gram.a_local_dense()
    }

    #[test]
    fn arrowhead_local_solve_matches_dense_without_market() {
        let mut jac = random_jacobian(4, 3, 3);
        for i in 0..jac.n_goods() {
            for j in 0..jac.n_goods() {
                jac.market_r[(i, j)] = 0.0;
            }
        }
        for block in &mut jac.market_sigma {
            for i in 0..block.nrows() {
                for j in 0..block.ncols() {
                    block[(i, j)] = 0.0;
                }
            }
        }
        let dense = dense_gram(&jac);
        let gram = jac.gram();
        let a_local = a_local_dense(&gram);
        for i in 0..dense.nrows() {
            for j in 0..dense.ncols() {
                assert!(
                    (dense[(i, j)] - a_local[(i, j)]).abs() < 1e-10,
                    "A_local mismatch at ({i},{j}): {} vs {}",
                    dense[(i, j)],
                    a_local[(i, j)]
                );
            }
        }
        let n = jac.n();
        let b = Col::from_fn(n, |i| (i as f64).sin());
        let damp = damp(n);
        let mut gram = jac.gram();
        gram.add_diagonal_vector_in_place(&damp);
        let mut dense_damped = dense.clone();
        for i in 0..n {
            dense_damped[(i, i)] += damp[i];
        }
        let x = gram.solve_spd(&b).expect("local solve");
        let x_dense = dense_solve_spd(&dense_damped, &b);
        for i in 0..n {
            assert!((x[i] - x_dense[i]).abs() < 1e-5, "i={i}");
        }
    }

    #[test]
    fn dense_gram_is_spd_and_arrowhead_solve_matches() {
        let jac = random_jacobian(4, 3, 3);
        let dense = dense_gram(&jac);
        let n = jac.n();
        let b = Col::from_fn(n, |i| (i as f64).sin());
        let damp = damp(n);
        let mut gram = jac.gram();
        gram.add_diagonal_vector_in_place(&damp);
        let mut dense_damped = dense.clone();
        for i in 0..n {
            dense_damped[(i, i)] += damp[i];
        }
        let _ = dense_solve_spd(&dense_damped, &b);
        let x = gram.solve_spd(&b).expect("structured solve");
        let x_dense = dense_solve_spd(&dense_damped, &b);
        for i in 0..n {
            assert!((x[i] - x_dense[i]).abs() < 1e-5, "i={i}");
        }
    }

    #[test]
    fn solve_spd_matches_dense_cholesky() {
        let jac = random_jacobian(4, 3, 3);
        let mut gram = jac.gram();
        let dense = dense_gram(&jac);
        let b = Col::from_fn(gram.n(), |i| (i as f64).sin());
        let damp = damp(gram.n());
        gram.add_diagonal_vector_in_place(&damp);
        let mut dense_damped = dense.clone();
        for i in 0..gram.n() {
            dense_damped[(i, i)] += damp[i];
        }
        let x = gram.solve_spd(&b).expect("solve");
        let x_dense = dense_solve_spd(&dense_damped, &b);
        for i in 0..gram.n() {
            assert!(
                (x[i] - x_dense[i]).abs() < 1e-5,
                "i={i} got {} want {}",
                x[i],
                x_dense[i]
            );
        }
    }

    #[test]
    fn solve_spd_with_damping_matches_dense() {
        let jac = random_jacobian(4, 3, 4);
        let mut gram = jac.gram();
        let dense = dense_gram(&jac);
        let n = gram.n();
        let damp = Col::from_fn(n, |i| 0.01 * (i as f64 + 1.0) + 1e-4);
        gram.add_diagonal_vector_in_place(&damp);
        let mut dense_damped = dense.clone();
        for i in 0..n {
            dense_damped[(i, i)] += damp[i];
        }
        let b = Col::from_fn(n, |i| (i as f64 + 0.5).cos());
        let x = gram.solve_spd(&b).expect("solve");
        let x_dense = dense_solve_spd(&dense_damped, &b);
        for i in 0..n {
            assert!((x[i] - x_dense[i]).abs() < 1e-5, "i={i}");
        }
    }

    #[test]
    fn large_storage_stays_structured() {
        let g = 50;
        let s = 100;
        let jac = ArrowheadMat::zeros(g, s);
        let n = g * (1 + s);
        let structured = jac.structured_storage_len();
        let dense_n2 = n * n;
        assert!(
            structured < dense_n2 / 10,
            "structured={structured} should be ≪ dense n²={dense_n2}"
        );
        let gram = jac.gram();
        assert!(
            gram.structured_storage_len() < dense_n2 / 10,
            "gram storage should stay structured"
        );
    }

    #[test]
    #[ignore = "slow; run with `cargo test -p basin-arrowhead large_solve -- --ignored --release`"]
    fn large_solve_beats_sparse_cholesky_in_time() {
        let g = 50;
        let s = 100;
        let jac = random_jacobian(g, s, 99);
        let mut gram = jac.gram();
        let n = gram.n();
        let b = Col::from_fn(n, |i| (i as f64 * 0.01).sin());
        let damp = damp(n);
        gram.add_diagonal_vector_in_place(&damp);

        let t0 = std::time::Instant::now();
        for _ in 0..3 {
            let _ = gram.clone().solve_spd(&b).expect("structured");
        }
        let structured_ms = t0.elapsed().as_millis();

        let mut dense = dense_gram(&jac);
        for i in 0..n {
            dense[(i, i)] += damp[i];
        }
        let mut triplets = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                triplets.push(Triplet::new(i, j, dense[(i, j)]));
            }
        }
        let csc = SparseColMat::try_new_from_triplets(n, n, &triplets).expect("csc");
        let t1 = std::time::Instant::now();
        for _ in 0..3 {
            let _ = csc.clone().solve_spd(&b).expect("sparse");
        }
        let sparse_ms = t1.elapsed().as_millis();

        assert!(
            structured_ms * 5 < sparse_ms.max(1),
            "structured={structured_ms}ms sparse={sparse_ms}ms"
        );
    }

    // --- Property tests (dense oracle as ground truth) ---

    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;

    type PropResult = Result<(), TestCaseError>;

    fn assert_matvec_matches_dense(jac: &ArrowheadMat, x: &Col<f64>) -> PropResult {
        let j = jac.to_dense();
        let y = jac.matvec(x);
        let mut y_dense = Col::<f64>::zeros(jac.n());
        matmul(
            y_dense.as_mut().as_mat_mut(),
            Accum::Replace,
            j.as_ref(),
            x.as_mat(),
            1.0,
            Par::Seq,
        );
        for i in 0..jac.n() {
            prop_assert!((y[i] - y_dense[i]).abs() < 1e-11);
        }
        Ok(())
    }

    fn assert_mat_transpose_vec_matches_dense(jac: &ArrowheadMat, r: &Col<f64>) -> PropResult {
        let j = jac.to_dense();
        let g = jac.mat_transpose_vec(r);
        let mut g_dense = Col::<f64>::zeros(jac.n());
        matmul(
            g_dense.as_mut().as_mat_mut(),
            Accum::Replace,
            j.transpose(),
            r.as_mat(),
            1.0,
            Par::Seq,
        );
        for i in 0..jac.n() {
            prop_assert!((g[i] - g_dense[i]).abs() < 1e-11);
        }
        Ok(())
    }

    fn assert_solve_spd_matches_dense(
        jac: &ArrowheadMat,
        damp: &Col<f64>,
        b: &Col<f64>,
    ) -> PropResult {
        let mut gram = jac.gram();
        gram.add_diagonal_vector_in_place(damp);
        let x = gram.solve_spd(b).map_err(|_| {
            TestCaseError::fail("structured solve should succeed with sufficient damping")
        })?;
        let mut dense = dense_gram(jac);
        for i in 0..jac.n() {
            dense[(i, i)] += damp[i];
        }
        let x_dense = dense_solve_spd(&dense, b);
        for i in 0..jac.n() {
            prop_assert!((x[i] - x_dense[i]).abs() < 1e-5);
        }
        Ok(())
    }

    proptest! {
        /// `J x` and `Jᵀ r` agree with dense reference for random `(G, S)` shapes.
        #[test]
        fn proptest_matvec_ops_match_dense(
            g in 1usize..=5,
            s in 0usize..=4,
            seed in 0u64..10_000,
        ) {
            let jac = random_jacobian(g, s, seed);
            let n = jac.n();
            let x = Col::from_fn(n, |i| ((seed as f64 + i as f64) * 0.07).sin());
            let r = Col::from_fn(n, |i| ((seed as f64 - i as f64) * 0.05).cos());
            assert_matvec_matches_dense(&jac, &x)?;
            assert_mat_transpose_vec_matches_dense(&jac, &r)?;
        }

        /// Damped `solve_spd` matches dense Cholesky for random RHS and diagonal.
        #[test]
        fn proptest_solve_spd_matches_dense(
            g in 2usize..=5,
            s in 1usize..=4,
            seed in 0u64..10_000,
            log_damp in -3.0f64..0.0,
        ) {
            let jac = random_jacobian(g, s, seed);
            let n = jac.n();
            let base = 10f64.powf(log_damp);
            let damp = Col::from_fn(n, |i| base * (1.0 + 0.1 * i as f64));
            let b = Col::from_fn(n, |i| ((seed as f64 + i as f64) * 0.11).sin());
            assert_solve_spd_matches_dense(&jac, &damp, &b)?;
        }

        /// [`MaxDiagonal`] tracks the true Gram diagonal after damping.
        #[test]
        fn proptest_max_diagonal_matches_dense(
            g in 2usize..=5,
            s in 1usize..=4,
            seed in 0u64..10_000,
        ) {
            let jac = random_jacobian(g, s, seed);
            let mut gram = jac.gram();
            let damp = damp(gram.n());
            gram.add_diagonal_vector_in_place(&damp);
            let structured = gram.max_diagonal();
            let mut dense = dense_gram(&jac);
            for i in 0..gram.n() {
                dense[(i, i)] += damp[i];
            }
            let dense_max = (0..gram.n())
                .map(|i| dense[(i, i)])
                .fold(f64::NEG_INFINITY, f64::max);
            prop_assert!((structured - dense_max).abs() < 1e-10);
        }

        /// Storage stays `O(S·G²)`, not `O(n²)`, for Vic3-scale `(G, S)`.
        #[test]
        fn proptest_storage_stays_structured(g in 3usize..=20, s in 30usize..=50) {
            let jac = ArrowheadMat::zeros(g, s);
            let n = g * (1 + s);
            let structured = jac.structured_storage_len();
            prop_assert!(structured * 10 < n * n);
            let gram = jac.gram();
            prop_assert!(gram.structured_storage_len() * 10 < n * n);
        }
    }
}
