//! Resource tracks: ordered backlogs drained by a worker-pool rate.
//!
//! This module is the **framework-shaped** timing core. Domain code (Vic3
//! construction sectors, innovation, hire timers) supplies work amounts and
//! rates; this module only answers “how many days until job *k* finishes?”
//!
//! # Units
//!
//! - **Work** — abstract points remaining on a job (construction points,
//!   innovation cost, or a synthetic `1.0` per legacy fixed-day wait).
//! - **Rate** — work units completed per day by the track’s worker pool.
//! - **Days** — `ceil(work / rate)` as [`u32`], or [`None`] when `rate` is not
//!   strictly positive (no finite ETA).
//!
//! # Construction vs independent queues
//!
//! Construction is **one backlog** with many workers contributing to a shared
//! rate — not N independent queues. Research/law/hire are typically
//! single-head tracks (`max_inflight = 1` at the host layer).
//!
//! # Examples
//!
//! ```
//! use vic3_planning::tracks::{eta_days, eta_prefix_days, Backlog, Job, TrackId};
//!
//! let construction = TrackId::Construction;
//! let backlog = Backlog::from_jobs([
//!     Job::new(construction, "barn", 100.0),
//!     Job::new(construction, "mill", 50.0),
//! ]);
//! let rate = 10.0;
//! assert_eq!(eta_days(&backlog, rate, 0), Some(10)); // 100/10
//! assert_eq!(eta_prefix_days(&backlog, rate, 1), Some(15)); // (100+50)/10
//! ```

use serde::{Deserialize, Serialize};

/// Which shared resource backlog a job belongs to.
///
/// Host code maps Vic3 queues onto these ids. Construction shares one pool;
/// other variants are independent for heuristic max/sum purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TrackId {
    /// Government/private construction backlog (shared construction output).
    Construction,
    /// Single-head technology research.
    Research,
    /// Declared interest establishment.
    Interest,
    /// Military hire / crew training.
    Hire,
    /// Law enactment checkpoint.
    Law,
}

/// One unit of work on a track.
///
/// `work` must be finite and non-negative. Identity (`id`) is opaque to ETA
/// math and only used for diagnostics / matching completions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    /// Track this job is queued on.
    pub track: TrackId,
    /// Opaque job key (building type, tech id, …).
    pub id: String,
    /// Remaining work units (≥ 0, finite).
    pub work: f64,
}

impl Job {
    /// Build a job with the given remaining work.
    ///
    /// # Panics
    ///
    /// Panics if `work` is negative or non-finite.
    pub fn new(track: TrackId, id: impl Into<String>, work: f64) -> Self {
        assert!(
            work.is_finite() && work >= 0.0,
            "job work must be finite and ≥ 0, got {work}"
        );
        Self {
            track,
            id: id.into(),
            work,
        }
    }
}

/// Ordered jobs on one track (index 0 is the head).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Backlog {
    jobs: Vec<Job>,
}

impl Backlog {
    /// Empty backlog.
    pub fn new() -> Self {
        Self { jobs: Vec::new() }
    }

    /// Backlog from an iterator of jobs (order preserved).
    pub fn from_jobs(jobs: impl IntoIterator<Item = Job>) -> Self {
        Self {
            jobs: jobs.into_iter().collect(),
        }
    }

    /// Number of queued jobs.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Whether the backlog is empty.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Borrow the ordered jobs.
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// Append a job at the tail.
    pub fn push(&mut self, job: Job) {
        self.jobs.push(job);
    }

    /// Head job, if any.
    pub fn head(&self) -> Option<&Job> {
        self.jobs.first()
    }
}

/// Worker-pool throughput for a track (work units per day).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WorkerPool {
    /// Work units completed per day. Must be finite; ≤ 0 means no finite ETA.
    pub rate: f64,
}

impl WorkerPool {
    /// Pool with the given daily rate.
    pub fn new(rate: f64) -> Self {
        Self { rate }
    }

    /// Whether [`eta_days`] can return a finite value for positive work.
    pub fn can_make_progress(&self) -> bool {
        self.rate.is_finite() && self.rate > 0.0
    }
}

/// Days to finish the job at `index` ignoring jobs behind it
/// (`ceil(job.work / rate)`).
///
/// Returns [`None`] when `index` is out of range, `rate` is not strictly
/// positive, or work is non-finite.
///
/// Zero work completes in **0** days.
pub fn eta_days(backlog: &Backlog, rate: f64, index: usize) -> Option<u32> {
    let job = backlog.jobs.get(index)?;
    days_for_work(job.work, rate)
}

/// Days until the job at `index` finishes, including all work ahead of it
/// (`ceil(prefix_sum / rate)`).
///
/// This is the construction-style ETA: many workers drain one ordered queue.
pub fn eta_prefix_days(backlog: &Backlog, rate: f64, index: usize) -> Option<u32> {
    if index >= backlog.jobs.len() {
        return None;
    }
    let mut prefix = 0.0;
    for job in backlog.jobs.iter().take(index + 1) {
        if !job.work.is_finite() || job.work < 0.0 {
            return None;
        }
        prefix += job.work;
    }
    days_for_work(prefix, rate)
}

/// Days until the head job completes (`eta_prefix_days(..., 0)`).
pub fn eta_head_days(backlog: &Backlog, rate: f64) -> Option<u32> {
    if backlog.is_empty() {
        None
    } else {
        eta_prefix_days(backlog, rate, 0)
    }
}

/// Convert work + rate into whole days.
///
/// Policy: `ceil(work / rate)` as `u32`, saturating at [`u32::MAX`].
/// `work == 0` → `Some(0)`. Non-positive or non-finite `rate` → [`None`].
pub fn days_for_work(work: f64, rate: f64) -> Option<u32> {
    if !work.is_finite() || work < 0.0 {
        return None;
    }
    if work == 0.0 {
        return Some(0);
    }
    if !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    let days = (work / rate).ceil();
    if !days.is_finite() || days < 0.0 {
        return None;
    }
    if days >= f64::from(u32::MAX) {
        Some(u32::MAX)
    } else {
        Some(days as u32)
    }
}

/// Map a legacy fixed-day duration to synthetic work with rate `1.0`.
///
/// `constant_rate_days(d)` satisfies `days_for_work(d as f64, 1.0) == Some(d)`
/// for ordinary day counts.
pub fn constant_rate_work(days: u16) -> f64 {
    f64::from(days)
}

/// Rate used with [`constant_rate_work`] so ETA equals the day count.
pub const CONSTANT_RATE: f64 = 1.0;

/// One in-flight track with its backlog and rate, for multi-track selection.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackState {
    /// Track identity.
    pub id: TrackId,
    /// Ordered jobs.
    pub backlog: Backlog,
    /// Shared worker rate for this track.
    pub rate: f64,
}

/// Earliest head-completion among non-empty tracks.
///
/// Returns `(track, days)` for the minimum finite head ETA. Ties break by
/// [`TrackId`] ord. Empty backlogs and non-positive rates are skipped.
pub fn next_completion(tracks: &[TrackState]) -> Option<(TrackId, u32)> {
    let mut best: Option<(TrackId, u32)> = None;
    for track in tracks {
        let Some(days) = eta_head_days(&track.backlog, track.rate) else {
            continue;
        };
        match best {
            None => best = Some((track.id, days)),
            Some((best_id, best_days)) => {
                if days < best_days || (days == best_days && track.id < best_id) {
                    best = Some((track.id, days));
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn eta_empty_is_none() {
        let b = Backlog::new();
        assert_eq!(eta_days(&b, 1.0, 0), None);
        assert_eq!(eta_head_days(&b, 1.0), None);
    }

    #[test]
    fn eta_one_job() {
        let b = Backlog::from_jobs([Job::new(TrackId::Construction, "a", 100.0)]);
        assert_eq!(eta_days(&b, 10.0, 0), Some(10));
        assert_eq!(eta_prefix_days(&b, 10.0, 0), Some(10));
    }

    #[test]
    fn eta_many_prefix_sums() {
        let b = Backlog::from_jobs([
            Job::new(TrackId::Construction, "a", 100.0),
            Job::new(TrackId::Construction, "b", 50.0),
            Job::new(TrackId::Construction, "c", 25.0),
        ]);
        assert_eq!(eta_prefix_days(&b, 10.0, 0), Some(10));
        assert_eq!(eta_prefix_days(&b, 10.0, 1), Some(15));
        assert_eq!(eta_prefix_days(&b, 10.0, 2), Some(18)); // ceil(175/10)=18
    }

    #[test]
    fn rate_zero_yields_none() {
        let b = Backlog::from_jobs([Job::new(TrackId::Research, "t", 10.0)]);
        assert_eq!(eta_days(&b, 0.0, 0), None);
        assert_eq!(eta_days(&b, -1.0, 0), None);
        assert_eq!(days_for_work(10.0, f64::NAN), None);
    }

    #[test]
    fn zero_work_is_zero_days() {
        assert_eq!(days_for_work(0.0, 5.0), Some(0));
    }

    #[test]
    fn constant_rate_adapter_matches_days() {
        for days in [0u16, 1, 7, 180, 365] {
            let work = constant_rate_work(days);
            assert_eq!(days_for_work(work, CONSTANT_RATE), Some(u32::from(days)));
        }
    }

    #[test]
    fn next_completion_picks_earliest() {
        let tracks = [
            TrackState {
                id: TrackId::Research,
                backlog: Backlog::from_jobs([Job::new(TrackId::Research, "t", 40.0)]),
                rate: CONSTANT_RATE,
            },
            TrackState {
                id: TrackId::Construction,
                backlog: Backlog::from_jobs([Job::new(TrackId::Construction, "b", 10.0)]),
                rate: 1.0,
            },
        ];
        assert_eq!(next_completion(&tracks), Some((TrackId::Construction, 10)));
    }

    #[test]
    fn next_completion_skips_empty_and_zero_rate() {
        let tracks = [
            TrackState {
                id: TrackId::Hire,
                backlog: Backlog::new(),
                rate: 1.0,
            },
            TrackState {
                id: TrackId::Law,
                backlog: Backlog::from_jobs([Job::new(TrackId::Law, "l", 5.0)]),
                rate: 0.0,
            },
            TrackState {
                id: TrackId::Interest,
                backlog: Backlog::from_jobs([Job::new(TrackId::Interest, "i", 9.0)]),
                rate: CONSTANT_RATE,
            },
        ];
        assert_eq!(next_completion(&tracks), Some((TrackId::Interest, 9)));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn prefix_eta_nondecreasing_in_index(
            works in prop::collection::vec(1.0f64..1_000.0, 1..8),
            rate in 0.1f64..50.0,
        ) {
            let backlog = Backlog::from_jobs(works.into_iter().enumerate().map(|(i, w)| {
                Job::new(TrackId::Construction, format!("j{i}"), w)
            }));
            let mut prev = 0u32;
            for i in 0..backlog.len() {
                let eta = eta_prefix_days(&backlog, rate, i).expect("finite rate");
                prop_assert!(eta >= prev, "eta[{i}]={eta} < prev={prev}");
                prev = eta;
            }
        }

        #[test]
        fn eta_scales_inverse_with_rate(
            work in 1.0f64..500.0,
            rate in 0.5f64..20.0,
        ) {
            let slow = days_for_work(work, rate).unwrap();
            let fast = days_for_work(work, rate * 2.0).unwrap();
            prop_assert!(fast <= slow);
            // ceil(w/r) vs ceil(w/(2r)): roughly half, allow +1 for ceil noise
            prop_assert!(slow <= fast.saturating_mul(2).saturating_add(1));
        }
    }
}
