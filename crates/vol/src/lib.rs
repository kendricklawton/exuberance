//! Volatility math for the `exuberance` trading engine.
//!
//! Everything here is pure and deterministic so it can be tested offline and
//! reused by the `signals` crate, the CLI, and (eventually) live scanners.
//!
//! The strategy this supports: find **cheap implied volatility on names with a
//! proven ability to move** — i.e. options underpricing future movement
//! (a positive *variance risk premium* for the buyer).

/// Trading days in a year — the standard annualization factor for daily bars.
pub const TRADING_DAYS_PER_YEAR: f64 = 252.0;

/// Natural-log returns from a price series: `ln(p[i] / p[i-1])`.
///
/// Returns a vector of length `prices.len() - 1` (empty if fewer than 2 prices).
/// Prices are assumed positive: validating bad ticks (zero/negative prices) is the
/// data-adapter's job (ROADMAP P7.2), not this pure-math layer's.
pub fn log_returns(prices: &[f64]) -> Vec<f64> {
    if prices.len() < 2 {
        return Vec::new();
    }
    prices.windows(2).map(|w| (w[1] / w[0]).ln()).collect()
}

/// Simple (arithmetic) returns from a price series: `p[i]/p[i-1] - 1`.
pub fn simple_returns(prices: &[f64]) -> Vec<f64> {
    if prices.len() < 2 {
        return Vec::new();
    }
    prices.windows(2).map(|w| w[1] / w[0] - 1.0).collect()
}

/// Sample standard deviation (n-1 denominator). `None` if fewer than 2 samples.
pub fn sample_std_dev(xs: &[f64]) -> Option<f64> {
    if xs.len() < 2 {
        return None;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(var.sqrt())
}

/// Annualized realized (historical) volatility from a daily price series.
///
/// Standard deviation of daily log returns, scaled by `sqrt(periods_per_year)`.
/// Returned as a decimal (0.30 == 30% annualized). `None` if not enough data.
pub fn realized_vol(prices: &[f64], periods_per_year: f64) -> Option<f64> {
    let rets = log_returns(prices);
    sample_std_dev(&rets).map(|sd| sd * periods_per_year.sqrt())
}

/// Convenience: annualized realized vol from daily bars (252 trading days).
pub fn realized_vol_daily(prices: &[f64]) -> Option<f64> {
    realized_vol(prices, TRADING_DAYS_PER_YEAR)
}

/// IV Rank: where `current` sits within the [min, max] of its own history.
///
/// `0.0` == at the historical low, `1.0` == at the historical high. This is the
/// "cheap *for this name*" gauge — an absolute IV of 30 is meaningless without it.
/// `None` if history is empty or perfectly flat (no range to rank within).
pub fn iv_rank(current: f64, history: &[f64]) -> Option<f64> {
    if history.is_empty() {
        return None;
    }
    let min = history.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = history.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    if range <= f64::EPSILON {
        return None;
    }
    Some(((current - min) / range).clamp(0.0, 1.0))
}

/// IV Percentile: fraction of historical observations strictly below `current`.
///
/// `0.10` means today's IV is cheaper than 90% of its own history. More robust
/// to outliers than [`iv_rank`]. Range `[0.0, 1.0]`; `0.0` if history is empty.
pub fn iv_percentile(current: f64, history: &[f64]) -> f64 {
    if history.is_empty() {
        return 0.0;
    }
    let below = history.iter().filter(|&&h| h < current).count();
    below as f64 / history.len() as f64
}

/// Implied − Realized spread. **Negative** means implied is below realized —
/// the market is underpricing movement (the setup we want as an option *buyer*).
pub fn implied_realized_spread(implied_vol: f64, realized_vol: f64) -> f64 {
    implied_vol - realized_vol
}

/// Ratio of realized to implied vol. `> 1.0` means the stock has actually moved
/// *more* than options currently imply — cheap vol on a proven mover.
/// `None` if `implied_vol` is non-positive.
pub fn realized_over_implied(realized_vol: f64, implied_vol: f64) -> Option<f64> {
    if implied_vol <= 0.0 {
        return None;
    }
    Some(realized_vol / implied_vol)
}

/// Largest single-period absolute simple return in the series (as a decimal).
/// `None` if fewer than 2 prices. `0.12` == a 12% one-day move occurred.
pub fn max_abs_move(prices: &[f64]) -> Option<f64> {
    let rets = simple_returns(prices);
    rets.iter()
        .map(|r| r.abs())
        .fold(None, |acc, r| Some(acc.map_or(r, |a: f64| a.max(r))))
}

/// Count single-period moves whose absolute return meets or exceeds `threshold`.
/// `threshold` is a decimal (0.10 == 10%). This is the "proven mover" filter.
pub fn count_moves_over(prices: &[f64], threshold: f64) -> usize {
    simple_returns(prices)
        .iter()
        .filter(|r| r.abs() >= threshold)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn log_returns_basic() {
        let r = log_returns(&[100.0, 110.0]);
        assert_eq!(r.len(), 1);
        assert!(approx(r[0], (1.1_f64).ln(), 1e-12));
        assert!(log_returns(&[100.0]).is_empty());
        assert!(log_returns(&[]).is_empty());
    }

    #[test]
    fn simple_returns_basic() {
        // 100 → 110 is exactly +10%; 110 → 99 is exactly −10% (99/110 == 0.9).
        let r = simple_returns(&[100.0, 110.0, 99.0]);
        assert_eq!(r.len(), 2);
        assert!(approx(r[0], 0.10, 1e-12));
        assert!(approx(r[1], -0.10, 1e-12));
        assert!(simple_returns(&[100.0]).is_empty());
        assert!(simple_returns(&[]).is_empty());
    }

    #[test]
    fn sample_std_dev_known_answer() {
        // Classic hand-computed set: mean 5, squared deviations sum 32,
        // sample variance 32/7 → sd = sqrt(32/7).
        let xs = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let sd = sample_std_dev(&xs).unwrap();
        assert!(approx(sd, (32.0_f64 / 7.0).sqrt(), 1e-12));
        // Fewer than 2 samples has no spread to measure.
        assert!(sample_std_dev(&[1.0]).is_none());
        assert!(sample_std_dev(&[]).is_none());
    }

    #[test]
    fn realized_vol_is_positive_and_scales() {
        // A gently oscillating series has non-zero, finite realized vol.
        let prices = [100.0, 101.0, 100.0, 102.0, 101.0, 103.0];
        let rv = realized_vol_daily(&prices).unwrap();
        assert!(rv > 0.0 && rv.is_finite());
        // A flat series has zero realized vol.
        let flat = [100.0, 100.0, 100.0, 100.0];
        assert!(approx(realized_vol_daily(&flat).unwrap(), 0.0, 1e-12));
    }

    #[test]
    fn realized_vol_known_answer() {
        // 100 → 110 → 100: log returns are exactly [ln 1.1, −ln 1.1] (ln(10/11) == −ln 1.1),
        // so mean = 0, sample variance = 2·r²/1, sd = r·√2, and annualized
        // rv = r·√2·√252 = r·√504 — derived by hand, independent of the implementation.
        let expected = (1.1_f64).ln() * (504.0_f64).sqrt();
        let rv = realized_vol_daily(&[100.0, 110.0, 100.0]).unwrap();
        assert!(approx(rv, expected, 1e-9), "rv {rv} != expected {expected}");
    }

    #[test]
    fn iv_rank_endpoints_and_mid() {
        let hist = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert!(approx(iv_rank(10.0, &hist).unwrap(), 0.0, 1e-12));
        assert!(approx(iv_rank(50.0, &hist).unwrap(), 1.0, 1e-12));
        assert!(approx(iv_rank(30.0, &hist).unwrap(), 0.5, 1e-12));
        // Flat history has no range → None.
        assert!(iv_rank(30.0, &[30.0, 30.0]).is_none());
        assert!(iv_rank(30.0, &[]).is_none());
    }

    #[test]
    fn iv_percentile_counts_below() {
        let hist = [10.0, 20.0, 30.0, 40.0];
        assert!(approx(iv_percentile(25.0, &hist), 0.5, 1e-12)); // 2 of 4 below
        assert!(approx(iv_percentile(5.0, &hist), 0.0, 1e-12));
        assert!(approx(iv_percentile(100.0, &hist), 1.0, 1e-12));
    }

    #[test]
    fn spread_and_ratio_flag_cheap_vol() {
        // implied 20%, realized 35% → implied underprices movement.
        // Exact values: spread = 0.20 − 0.35 = −0.15; ratio = 0.35 / 0.20 = 1.75.
        assert!(approx(implied_realized_spread(0.20, 0.35), -0.15, 1e-12));
        assert!(approx(
            realized_over_implied(0.35, 0.20).unwrap(),
            1.75,
            1e-12
        ));
        assert!(realized_over_implied(0.35, 0.0).is_none());
        assert!(realized_over_implied(0.35, -0.1).is_none());
    }

    #[test]
    fn move_detection() {
        // +12%, -20%, +2% — moves chosen off the exact threshold so the
        // `>= threshold` comparison isn't at the mercy of float rounding.
        let prices = [100.0, 112.0, 89.6, 91.39];
        assert!(approx(max_abs_move(&prices).unwrap(), 0.20, 1e-9));
        assert_eq!(count_moves_over(&prices, 0.10), 2); // +12% and -20%
        assert_eq!(count_moves_over(&prices, 0.15), 1); // only -20%
    }
}
