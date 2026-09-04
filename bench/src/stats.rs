//! Percentile reporting.
//!
//! Means hide exactly the thing that matters for a viewer: the slow frame the
//! user actually notices. Every measurement in the spike is reported as a
//! distribution, and the pass/fail call is made on p95, never on the mean.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub n: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
}

impl Stats {
    pub fn from_millis(mut v: Vec<f64>) -> Self {
        assert!(!v.is_empty(), "stats over an empty sample");
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = v.len();
        let pick = |q: f64| -> f64 {
            // Nearest-rank percentile: no interpolation between samples, so a
            // reported p95 is always a measurement that actually happened.
            let idx = ((q * n as f64).ceil() as usize)
                .saturating_sub(1)
                .min(n - 1);
            v[idx]
        };
        Stats {
            n,
            min_ms: v[0],
            p50_ms: pick(0.50),
            p95_ms: pick(0.95),
            p99_ms: pick(0.99),
            max_ms: v[n - 1],
            mean_ms: v.iter().sum::<f64>() / n as f64,
        }
    }

    pub fn one(ms: f64) -> Self {
        Self::from_millis(vec![ms])
    }
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "n={:<4} p50={:>8.2}ms  p95={:>8.2}ms  p99={:>8.2}ms  max={:>8.2}ms",
            self.n, self.p50_ms, self.p95_ms, self.p99_ms, self.max_ms
        )
    }
}
