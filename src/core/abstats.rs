//! `abstats` — distribution-free statistics for a blinded, interleaved A/B mouse-firmware trial.
//!
//! Zero dependencies (core + std only). Everything here is deterministic except the
//! Monte-Carlo fallback in [`perm_test`], which uses a seeded xorshift PRNG.
//!
//! Design context this module is written for:
//!   * two conditions A and B, results withheld until the run ends;
//!   * trials interleaved in pairs A1,B1,A2,B2,... so a PAIRED analysis is available;
//!   * n per condition is small (6..20);
//!   * data are skewed (clicks-per-second) or small discrete integers with heavy ties
//!     (total actuation counts for the "weak input" variant).
//!
//! Reference-matching notes are attached to each function. Where behaviour matches
//! `scipy.stats` it is stated explicitly, because the verification harness compares
//! against scipy 1.18.1 / numpy 2.5.2.

// The full API of this module is kept, not just the part the interface reads
// today. These are self-contained numerical routines checked as a whole against
// an outside reference, and trimming them to the current call sites would make
// that check harder to repeat than the unused functions are worth.
#![allow(dead_code)]

#![allow(clippy::needless_range_loop)]

use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// 1. Normal tail (no dependency on libm / erfc from std, which does not exist)
// ---------------------------------------------------------------------------

/// erf(x) via the all-positive confluent series
/// `erf(x) = 2x e^{-x^2}/sqrt(pi) * sum_n (2x^2)^n / (1*3*...*(2n+1))`.
/// No cancellation; used for |x| < 2.
fn erf_series(x: f64) -> f64 {
    let x2 = x * x;
    let mut term = 1.0f64;
    let mut sum = 1.0f64;
    for n in 1..500usize {
        term *= 2.0 * x2 / ((2 * n + 1) as f64);
        sum += term;
        if term <= 1e-19 * sum {
            break;
        }
    }
    2.0 * x * (-x2).exp() * sum / PI.sqrt()
}

/// erfc(x) for x >= 2 via the Lentz-evaluated continued fraction
/// `erfc(x) = e^{-x^2}/sqrt(pi) * 1/(x + (1/2)/(x + 1/(x + (3/2)/(x + 2/(x + ...)))))`.
fn erfc_cf(x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let mut f = TINY;
    let mut c = f;
    let mut d = 0.0f64;
    for j in 1..2000usize {
        let a = if j == 1 { 1.0 } else { (j - 1) as f64 / 2.0 };
        let b = x;
        d = b + a * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + a / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() < 1e-17 {
            break;
        }
    }
    (-x * x).exp() * f / PI.sqrt()
}

/// Complementary error function, full range. Max relative error vs
/// `scipy.special.erfc` measured on this host: see the verification report.
pub fn erfc(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x < 0.0 {
        return 2.0 - erfc(-x);
    }
    if x < 2.0 {
        1.0 - erf_series(x)
    } else {
        erfc_cf(x)
    }
}

/// Upper tail of the standard normal, `P(Z >= z)`.
pub fn norm_sf(z: f64) -> f64 {
    0.5 * erfc(z / std::f64::consts::SQRT_2)
}

/// Two-sided normal p-value, `2 * P(Z >= |z|)`, clipped to [0, 1].
pub fn norm_two_sided(z: f64) -> f64 {
    let p = erfc(z.abs() / std::f64::consts::SQRT_2);
    p.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// 2. Ranks, order statistics, robust spread
// ---------------------------------------------------------------------------

/// Mid-ranks ("average" method, identical to `scipy.stats.rankdata(x, 'average')`).
///
/// Returns `(ranks, tie_group_sizes)` where `tie_group_sizes` lists the size of every
/// group of exactly-equal values (including singletons). The tie correction term used
/// everywhere below is `sum(t^3 - t)` over that list; singletons contribute 0.
pub fn mid_ranks(x: &[f64]) -> (Vec<f64>, Vec<usize>) {
    let n = x.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&i, &j| x[i].partial_cmp(&x[j]).expect("NaN in mid_ranks input"));
    let mut ranks = vec![0.0f64; n];
    let mut ties = Vec::new();
    let mut i = 0usize;
    while i < n {
        let mut j = i + 1;
        while j < n && x[idx[j]] == x[idx[i]] {
            j += 1;
        }
        let g = j - i;
        // ranks i+1 .. j (1-based) all get the average
        let avg = ((i + 1) as f64 + j as f64) / 2.0;
        for k in i..j {
            ranks[idx[k]] = avg;
        }
        ties.push(g);
        i = j;
    }
    (ranks, ties)
}

/// `sum(t^3 - t)` over tie group sizes.
pub fn tie_term(ties: &[usize]) -> f64 {
    ties.iter()
        .map(|&t| {
            let t = t as f64;
            t * t * t - t
        })
        .sum()
}

fn sorted_copy(x: &[f64]) -> Vec<f64> {
    let mut v = x.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).expect("NaN in sample"));
    v
}

/// Quantile by **linear interpolation of order statistics**, i.e. Hyndman & Fan
/// *type 7* — the default of `numpy.quantile` and of R's `quantile()`.
///
/// `h = (n - 1) * p`; result = `x[floor(h)] + (h - floor(h)) * (x[floor(h)+1] - x[floor(h)])`.
pub fn quantile_type7(x: &[f64], p: f64) -> f64 {
    assert!(!x.is_empty(), "quantile of empty sample");
    assert!((0.0..=1.0).contains(&p), "p out of range");
    let s = sorted_copy(x);
    let n = s.len();
    if n == 1 {
        return s[0];
    }
    let h = (n as f64 - 1.0) * p;
    let lo = h.floor();
    let li = lo as usize;
    if li + 1 >= n {
        return s[n - 1];
    }
    s[li] + (h - lo) * (s[li + 1] - s[li])
}

/// Median. Even n -> mean of the two central order statistics (== `quantile_type7(x, 0.5)`).
pub fn median(x: &[f64]) -> f64 {
    let s = sorted_copy(x);
    let n = s.len();
    assert!(n > 0, "median of empty sample");
    if n % 2 == 1 {
        s[n / 2]
    } else {
        0.5 * (s[n / 2 - 1] + s[n / 2])
    }
}

/// Median absolute deviation, scaled by the normal-consistency constant 1.4826
/// so that it estimates sigma for Gaussian data. Pass `scale = 1.0` for the raw MAD.
pub const MAD_NORMAL_CONSISTENCY: f64 = 1.4826;

pub fn mad(x: &[f64], scale: f64) -> f64 {
    let m = median(x);
    let dev: Vec<f64> = x.iter().map(|v| (v - m).abs()).collect();
    scale * median(&dev)
}

/// Interquartile range using type-7 quantiles (matches `numpy.percentile(x,[25,75])`).
pub fn iqr_type7(x: &[f64]) -> f64 {
    quantile_type7(x, 0.75) - quantile_type7(x, 0.25)
}

/// Everything the UI needs to describe one condition's spread.
#[derive(Debug, Clone, Copy)]
pub struct RobustSummary {
    pub n: usize,
    pub median: f64,
    pub q1: f64,
    pub q3: f64,
    pub iqr: f64,
    pub mad: f64,
    pub min: f64,
    pub max: f64,
}

pub fn robust_summary(x: &[f64]) -> RobustSummary {
    let s = sorted_copy(x);
    RobustSummary {
        n: x.len(),
        median: median(x),
        q1: quantile_type7(x, 0.25),
        q3: quantile_type7(x, 0.75),
        iqr: iqr_type7(x),
        mad: mad(x, MAD_NORMAL_CONSISTENCY),
        min: s[0],
        max: s[s.len() - 1],
    }
}

// ---------------------------------------------------------------------------
// 3. Mann-Whitney U / Wilcoxon rank-sum (unpaired)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MwuMethod {
    /// Project default: exact whenever there are no ties and `n1 + n2 <= EXACT_MWU_MAX_N`.
    /// This is *more* exact than scipy's `'auto'` (which only goes exact when
    /// `min(n1,n2) <= 8`), and is the right default for n in 6..20.
    Auto,
    /// Bit-for-bit reproduction of `scipy.stats.mannwhitneyu(method='auto')`:
    /// exact iff `(n1 <= 8 or n2 <= 8)` **and** there are no ties. Provided so the
    /// verification harness can compare like with like.
    AutoScipy,
    Exact,
    Asymptotic,
}

/// Largest pooled sample size for which the exact null distribution is built.
/// `C(60,30) ≈ 1.18e17` slightly exceeds 2^53, so counts above N=53 carry ~1e-16
/// relative rounding; probabilities remain accurate to ~1e-15.
pub const EXACT_MWU_MAX_N: usize = 60;

#[derive(Debug, Clone, Copy)]
pub struct MwuResult {
    /// `U_A = R_A - n1(n1+1)/2`, the number of (a,b) pairs with a > b, ties counted 1/2.
    /// This is the statistic `scipy.stats.mannwhitneyu(a, b).statistic` returns.
    pub u_a: f64,
    /// `U_B = n1*n2 - U_A`.
    pub u_b: f64,
    pub n_a: usize,
    pub n_b: usize,
    pub p_two_sided: f64,
    /// z used by the asymptotic branch (NaN when the exact branch was taken).
    pub z: f64,
    pub exact: bool,
    pub had_ties: bool,
    /// Rank-biserial correlation, **signed so that positive means A tends to exceed B**:
    /// `r = 2*U_A/(n1*n2) - 1 = 1 - 2*U_B/(n1*n2) = (U_A - U_B)/(n1*n2)`.
    /// The formula quoted as `1 - 2U/(n1 n2)` is this same quantity with `U = U_B`;
    /// feeding it `U_A` flips the sign, which is the classic reporting bug.
    pub rank_biserial: f64,
    /// Common-language effect size / probability of superiority:
    /// `P(A > B) + 0.5 P(A == B) = U_A / (n1*n2)`.
    pub prob_superiority: f64,
}

/// Exact null pmf of `U` for sample sizes `(n1, n2)` with **no ties**, indexed by
/// `u = 0 ..= n1*n2`. Built by DP over the rank-sum of the size-`n1` subset:
/// `dp[k][s]` = number of size-k subsets of {1..i} summing to s.
pub fn mwu_null_pmf(n1: usize, n2: usize) -> Vec<f64> {
    let n = n1 + n2;
    assert!(n <= EXACT_MWU_MAX_N, "pooled n too large for exact MWU");
    // max rank sum for a size-n1 subset
    let smax = n1 * n - n1 * (n1.saturating_sub(1)) / 2;
    let mut dp = vec![vec![0.0f64; smax + 1]; n1 + 1];
    dp[0][0] = 1.0;
    for r in 1..=n {
        let kmax = n1.min(r);
        for k in (1..=kmax).rev() {
            for s in (r..=smax).rev() {
                let v = dp[k - 1][s - r];
                if v != 0.0 {
                    dp[k][s] += v;
                }
            }
        }
    }
    let umax = n1 * n2;
    let offset = n1 * (n1 + 1) / 2; // U = S - n1(n1+1)/2
    let mut pmf = vec![0.0f64; umax + 1];
    let mut total = 0.0f64;
    for u in 0..=umax {
        let s = u + offset;
        let c = if s <= smax { dp[n1][s] } else { 0.0 };
        pmf[u] = c;
        total += c;
    }
    for v in pmf.iter_mut() {
        *v /= total;
    }
    pmf
}

/// `P(U >= u)` under the exact null.
pub fn pmf_sf(pmf: &[f64], u: f64) -> f64 {
    let k = u.ceil() as isize;
    if k <= 0 {
        return 1.0;
    }
    let k = k as usize;
    if k >= pmf.len() {
        return 0.0;
    }
    pmf[k..].iter().sum()
}

/// `P(U <= u)` under the exact null.
pub fn pmf_cdf(pmf: &[f64], u: f64) -> f64 {
    let k = u.floor() as isize;
    if k < 0 {
        return 0.0;
    }
    let k = (k as usize).min(pmf.len() - 1);
    pmf[..=k].iter().sum()
}

/// Two-sided Mann-Whitney U test.
///
/// Statistic and p-value conventions follow `scipy.stats.mannwhitneyu`:
///   * `U_A = R_A - n1(n1+1)/2` where `R_A` is the sum of the mid-ranks of `a`
///     in the pooled sample;
///   * exact two-sided p = `2 * P(U >= max(U_A, U_B))`, clipped to 1;
///   * asymptotic z uses `U_A`: `mu = n1 n2 / 2`,
///     `sigma = sqrt(n1 n2/12 * ((N+1) - sum(t^3-t)/(N(N-1))))`,
///     continuity correction subtracts `0.5 * sign(U_A - mu)` from the numerator
///     (sign chosen so the p-value always *increases*), p = `2 * P(Z >= |z|)`.
pub fn mann_whitney_u(a: &[f64], b: &[f64], method: MwuMethod) -> MwuResult {
    let n1 = a.len();
    let n2 = b.len();
    assert!(n1 > 0 && n2 > 0, "empty sample");
    let n = n1 + n2;

    let mut pooled = Vec::with_capacity(n);
    pooled.extend_from_slice(a);
    pooled.extend_from_slice(b);
    let (ranks, ties) = mid_ranks(&pooled);
    let r_a: f64 = ranks[..n1].iter().sum();
    let u_a = r_a - (n1 * (n1 + 1)) as f64 / 2.0;
    let u_b = (n1 * n2) as f64 - u_a;
    let had_ties = ties.iter().any(|&t| t > 1);

    let use_exact = match method {
        // Above the cap the null distribution cannot be enumerated in a
        // sensible amount of memory, so an explicit request for the exact
        // branch degrades to the approximation. A statistics library driving an
        // interface must not abort on the size of the data it was given.
        MwuMethod::Exact => n <= EXACT_MWU_MAX_N,
        MwuMethod::Asymptotic => false,
        MwuMethod::Auto => !had_ties && n <= EXACT_MWU_MAX_N,
        MwuMethod::AutoScipy => !had_ties && (n1 <= 8 || n2 <= 8),
    };

    let (p, z) = if use_exact {
        let pmf = mwu_null_pmf(n1, n2);
        let umax = u_a.max(u_b);
        ((2.0 * pmf_sf(&pmf, umax)).clamp(0.0, 1.0), f64::NAN)
    } else {
        let mu = (n1 * n2) as f64 / 2.0;
        let nf = n as f64;
        let sigma = ((n1 * n2) as f64 / 12.0
            * ((nf + 1.0) - tie_term(&ties) / (nf * (nf - 1.0))))
            .sqrt();
        let mut num = u_a - mu;
        // two-sided continuity correction: always shrink |numerator| by 1/2
        let sign = if num > 0.0 {
            1.0
        } else if num < 0.0 {
            -1.0
        } else {
            0.0
        };
        num -= 0.5 * sign;
        let z = num / sigma;
        (norm_two_sided(z), z)
    };

    let mn = (n1 * n2) as f64;
    MwuResult {
        u_a,
        u_b,
        n_a: n1,
        n_b: n2,
        p_two_sided: p,
        z,
        exact: use_exact,
        had_ties,
        rank_biserial: 2.0 * u_a / mn - 1.0,
        prob_superiority: u_a / mn,
    }
}

// ---------------------------------------------------------------------------
// 4. Hodges-Lehmann shift + distribution-free CI (two-sample)
// ---------------------------------------------------------------------------

/// All pairwise differences `a_i - b_j`, sorted ascending. Length `n1*n2`.
pub fn pairwise_diffs(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut d = Vec::with_capacity(a.len() * b.len());
    for &x in a {
        for &y in b {
            d.push(x - y);
        }
    }
    d.sort_by(|p, q| p.partial_cmp(q).expect("NaN"));
    d
}

/// Hodges-Lehmann estimate of the shift **A minus B**: median of all `a_i - b_j`.
/// Positive => condition A reads higher than B.
pub fn hodges_lehmann_shift(a: &[f64], b: &[f64]) -> f64 {
    let d = pairwise_diffs(a, b);
    let m = d.len();
    if m % 2 == 1 {
        d[m / 2]
    } else {
        0.5 * (d[m / 2 - 1] + d[m / 2])
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ShiftCi {
    pub estimate: f64,
    pub lo: f64,
    pub hi: f64,
    /// Actual (>= nominal) coverage of the interval, because the underlying
    /// null distribution is discrete.
    pub achieved_level: f64,
    /// Number of order statistics trimmed from each tail (`= qwilcox(alpha/2) - 1`).
    pub k: usize,
    /// False when the pooled sample has ties: the interval is then conservative
    /// rather than exactly distribution-free.
    pub exact: bool,
}

/// Shared trim-count rule for the two distribution-free intervals.
///
/// `k = min{u : P(T <= u) >= alpha/2}` (i.e. R's `qwilcox`/`qsignrank`), forced to
/// at least 1 (R does the same when the sample is too small for any exclusion) and
/// capped so the interval never inverts. The interval is then the `k`-th smallest to
/// the `k`-th largest order statistic, 1-based -- i.e. `k-1` values trimmed per tail.
/// Returned `cum` is `P(T <= k-1)`, so the achieved coverage is `1 - 2*cum`.
///
/// Derivation: the exact two-sided test rejects iff `U <= k-1` or `U >= m-k+1`, and
/// `U(theta) = #{D_i > theta}`, so the acceptance set in theta is
/// `[D_(k), D_(m-k+1)]` (1-based order statistics of the differences).
fn ci_trim(pmf: &[f64], m: usize, alpha: f64) -> (usize, f64) {
    let half = alpha / 2.0;
    let mut k = 0usize;
    let mut cum = 0.0f64;
    while k < pmf.len() {
        let next = cum + pmf[k];
        if next <= half {
            cum = next;
            k += 1;
        } else {
            break;
        }
    }
    if k == 0 {
        k = 1;
    }
    let kmax = m.div_ceil(2);
    if k > kmax {
        k = kmax.max(1);
    }
    let cum = pmf[..k.min(pmf.len())].iter().sum::<f64>();
    (k, cum)
}

/// Distribution-free CI for the location shift A-B, obtained by inverting the exact
/// two-sided Mann-Whitney test (Moses interval; Hollander & Wolfe §4.2).
///
/// See `ci_trim`: with `k = qwilcox(alpha/2, n1, n2)` the interval is
/// `[D_(k), D_(mn-k+1)]` over the sorted pairwise differences, i.e. `k-1` trimmed
/// per tail. Achieved coverage is `1 - 2*P(U <= k-1)`. This reproduces R's
/// `wilcox.test(x, y, conf.int=TRUE)` interval.
///
/// With ties present the exact null is no longer correct; the interval is still
/// reported (it is conservative) but `exact` is set false.
pub fn hodges_lehmann_shift_ci(a: &[f64], b: &[f64], alpha: f64) -> ShiftCi {
    let n1 = a.len();
    let n2 = b.len();
    // Above the exact cap the null distribution is not enumerated. Falling back
    // to the normal approximation keeps a usable interval; panicking here would
    // take down the interface for a long but perfectly ordinary run.
    if n1 + n2 > EXACT_MWU_MAX_N {
        return hodges_lehmann_shift_ci_normal(a, b, alpha);
    }
    let d = pairwise_diffs(a, b);
    let m = d.len();
    let pmf = mwu_null_pmf(n1, n2);

    let mut pooled = a.to_vec();
    pooled.extend_from_slice(b);
    let (_, ties) = mid_ranks(&pooled);
    let exact = !ties.iter().any(|&t| t > 1);

    let (k, cum) = ci_trim(&pmf, m, alpha);
    let est = if m % 2 == 1 {
        d[m / 2]
    } else {
        0.5 * (d[m / 2 - 1] + d[m / 2])
    };
    ShiftCi {
        estimate: est,
        lo: d[k - 1],
        hi: d[m - k],
        achieved_level: 1.0 - 2.0 * cum,
        k: k - 1,
        exact,
    }
}

// ---------------------------------------------------------------------------
// 5. Wilcoxon signed-rank (paired)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZeroMethod {
    /// Wilcoxon's original rule: discard zero differences, reduce n. scipy's default.
    /// Anti-conservative when zeros are common.
    Wilcox,
    /// Pratt (1959): rank |d| including the zeros, then drop the zeros from R+/R-.
    /// Recommended when ties between A and B are common (discrete actuation counts).
    Pratt,
    /// Split each zero's rank evenly between R+ and R-. Asymptotic only here.
    ZSplit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsrMethod {
    /// Project default: exact conditional (sign-flip) distribution whenever the number
    /// of ranked observations is <= `EXACT_WSR_MAX_N`, *including* when there are ties
    /// among |d| — the doubled-rank DP below handles ties exactly. Otherwise asymptotic.
    Auto,
    /// Reproduces `scipy.stats.wilcoxon(method='auto')`: n > 50 -> asymptotic;
    /// else no ties and no zeros -> exact; else n <= 13 -> exhaustive sign-flip
    /// permutation (numerically identical to our exact branch); else asymptotic.
    AutoScipy,
    Exact,
    Asymptotic,
}

pub const EXACT_WSR_MAX_N: usize = 50;

#[derive(Debug, Clone, Copy)]
pub struct WsrResult {
    pub r_plus: f64,
    pub r_minus: f64,
    /// scipy's reported `statistic` for a two-sided test: `min(r_plus, r_minus)`.
    pub statistic: f64,
    /// Number of ranked observations (n minus zeros under `Wilcox`, n otherwise).
    pub count: usize,
    pub n_zero: usize,
    pub p_two_sided: f64,
    pub z: f64,
    pub exact: bool,
    pub had_ties: bool,
    /// Matched-pairs rank-biserial correlation `(r_plus - r_minus)/(r_plus + r_minus)`.
    /// Positive => the A-minus-B differences are predominantly positive.
    pub rank_biserial: f64,
}

/// Exact null distribution of `2 * R+` under sign flipping of the supplied
/// (mid-)ranks. Doubling makes half-integer mid-ranks integral, so a single DP
/// handles the tied and untied cases identically. Returns the pmf indexed by
/// `2*R+ = 0 ..= sum(2*rank)`.
pub fn wsr_null_pmf_doubled(ranks: &[f64]) -> Vec<f64> {
    let doubled: Vec<usize> = ranks
        .iter()
        .map(|&r| {
            let d = r * 2.0;
            let di = d.round();
            debug_assert!((d - di).abs() < 1e-9, "mid-rank was not a multiple of 1/2");
            di as usize
        })
        .collect();
    let total: usize = doubled.iter().sum();
    let mut dp = vec![0.0f64; total + 1];
    dp[0] = 1.0;
    for &r in &doubled {
        for s in (r..=total).rev() {
            let v = dp[s - r];
            if v != 0.0 {
                dp[s] += v;
            }
        }
    }
    let norm: f64 = dp.iter().sum();
    for v in dp.iter_mut() {
        *v /= norm;
    }
    dp
}

/// Wilcoxon signed-rank test on the paired differences `d` (use `d[i] = a[i] - b[i]`).
///
/// Conventions match `scipy.stats.wilcoxon(d, zero_method=..., correction=..., method=...)`:
///   * ranks are mid-ranks of `|d|`;
///   * exact two-sided p = `2 * min(P(T >= R+), P(T <= R+))`, clipped to 1
///     (scipy rounds a non-integral R+ outward before the lookup; our exact branch
///     works on the doubled scale so no rounding is needed and no ties assumption
///     is made);
///   * asymptotic: `mu = m(m+1)/4`, `sigma = sqrt((m(m+1)(2m+1) - sum(t^3-t)/2)/24)`
///     with Cureton's Pratt adjustment when `ZeroMethod::Pratt`;
///     `correction` subtracts `sign(z)*0.5/sigma` from z.
pub fn wilcoxon_signed_rank(
    d: &[f64],
    zero_method: ZeroMethod,
    correction: bool,
    method: WsrMethod,
) -> WsrResult {
    let n = d.len();
    assert!(n > 0, "empty sample");
    let n_zero = d.iter().filter(|v| **v == 0.0).count();

    // Which observations get ranked?
    let ranked: Vec<f64> = match zero_method {
        ZeroMethod::Wilcox => d.iter().copied().filter(|v| *v != 0.0).collect(),
        _ => d.to_vec(),
    };
    let count = match zero_method {
        ZeroMethod::Wilcox => n - n_zero,
        _ => n,
    };
    if count == 0 {
        return WsrResult {
            r_plus: 0.0,
            r_minus: 0.0,
            statistic: 0.0,
            count: 0,
            n_zero,
            p_two_sided: 1.0,
            z: f64::NAN,
            exact: false,
            had_ties: false,
            rank_biserial: f64::NAN,
        };
    }

    let absd: Vec<f64> = ranked.iter().map(|v| v.abs()).collect();
    let (ranks, ties_all) = mid_ranks(&absd);

    let mut r_plus = 0.0f64;
    let mut r_minus = 0.0f64;
    for (i, &v) in ranked.iter().enumerate() {
        if v > 0.0 {
            r_plus += ranks[i];
        } else if v < 0.0 {
            r_minus += ranks[i];
        }
    }
    if zero_method == ZeroMethod::ZSplit {
        let rz: f64 = ranked
            .iter()
            .enumerate()
            .filter(|(_, v)| **v == 0.0)
            .map(|(i, _)| ranks[i])
            .sum();
        r_plus += rz / 2.0;
        r_minus += rz / 2.0;
    }

    // Tie groups among the *ranked non-zero* magnitudes, used to decide "has ties"
    // and, for Pratt, to drop the zero group from the tie correction.
    let mut ties_for_correction: Vec<usize> = ties_all.clone();
    if zero_method == ZeroMethod::Pratt && n_zero > 0 {
        // zeros are the smallest |d| so they form the first tie group
        // (mid_ranks returns groups in ascending value order)
        ties_for_correction[0] = 0;
    }
    let had_ties = {
        let nz: Vec<f64> = ranked.iter().filter(|v| **v != 0.0).map(|v| v.abs()).collect();
        let (_, t) = if nz.is_empty() {
            (vec![], vec![])
        } else {
            mid_ranks(&nz)
        };
        t.iter().any(|&g| g > 1)
    };

    let cf = count as f64;
    let mut mu = cf * (cf + 1.0) * 0.25;
    let mut se2 = cf * (cf + 1.0) * (2.0 * cf + 1.0);
    if zero_method == ZeroMethod::Pratt {
        let z0 = n_zero as f64;
        mu -= z0 * (z0 + 1.0) * 0.25;
        se2 -= z0 * (z0 + 1.0) * (2.0 * z0 + 1.0);
    }
    let sigma = ((se2 - tie_term(&ties_for_correction) / 2.0) / 24.0).sqrt();

    // number of sign-flippable observations (zeros never flip)
    let n_flip = ranked.iter().filter(|v| **v != 0.0).count();

    let use_exact = match method {
        WsrMethod::Exact => n_flip <= EXACT_WSR_MAX_N,
        WsrMethod::Asymptotic => false,
        WsrMethod::Auto => n_flip <= EXACT_WSR_MAX_N && zero_method != ZeroMethod::ZSplit,
        WsrMethod::AutoScipy => {
            if n > 50 {
                false
            } else if !had_ties && n_zero == 0 {
                true
            } else {
                n <= 13 // scipy switches to an exhaustive sign-flip permutation, same numbers
            }
        }
    };

    let (p, z) = {
        let mut zz = (r_plus - mu) / sigma;
        if correction {
            let sign = if zz > 0.0 {
                1.0
            } else if zz < 0.0 {
                -1.0
            } else {
                0.0
            };
            zz -= sign * 0.5 / sigma;
        }
        if use_exact {
            // Sign-flip null over the ranks of the NON-ZERO observations only.
            // Zeros contribute 0 to R+ under both Wilcox (they are absent) and
            // Pratt (they are ranked but excluded from R+), so the conditional
            // null is the same DP either way; only the ranks differ.
            let flip_ranks: Vec<f64> = ranked
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != 0.0)
                .map(|(i, _)| ranks[i])
                .collect();
            let pmf = wsr_null_pmf_doubled(&flip_ranks);
            let stat2 = (r_plus * 2.0).round();
            let sf: f64 = {
                let k = stat2.max(0.0) as usize;
                if k >= pmf.len() {
                    0.0
                } else {
                    pmf[k..].iter().sum()
                }
            };
            let cdf: f64 = {
                let k = (stat2.max(0.0) as usize).min(pmf.len() - 1);
                pmf[..=k].iter().sum()
            };
            ((2.0 * sf.min(cdf)).clamp(0.0, 1.0), zz)
        } else {
            (norm_two_sided(zz), zz)
        }
    };

    WsrResult {
        r_plus,
        r_minus,
        statistic: r_plus.min(r_minus),
        count,
        n_zero,
        p_two_sided: p,
        z,
        exact: use_exact,
        had_ties,
        rank_biserial: if r_plus + r_minus > 0.0 {
            (r_plus - r_minus) / (r_plus + r_minus)
        } else {
            f64::NAN
        },
    }
}

/// Walsh averages `(d_i + d_j)/2` for `i <= j`, sorted. Length `n(n+1)/2`.
pub fn walsh_averages(d: &[f64]) -> Vec<f64> {
    let n = d.len();
    let mut w = Vec::with_capacity(n * (n + 1) / 2);
    for i in 0..n {
        for j in i..n {
            w.push(0.5 * (d[i] + d[j]));
        }
    }
    w.sort_by(|a, b| a.partial_cmp(b).expect("NaN"));
    w
}

/// One-sample Hodges-Lehmann estimator: median of the Walsh averages of `d`.
/// For paired data `d = a - b` this estimates the median paired shift A-B.
pub fn hodges_lehmann_paired(d: &[f64]) -> f64 {
    let w = walsh_averages(d);
    let m = w.len();
    if m % 2 == 1 {
        w[m / 2]
    } else {
        0.5 * (w[m / 2 - 1] + w[m / 2])
    }
}

/// Exact null pmf of `R+` for `n` untied observations, indexed by `0 ..= n(n+1)/2`.
pub fn wsr_null_pmf_untied(n: usize) -> Vec<f64> {
    let ranks: Vec<f64> = (1..=n).map(|i| i as f64).collect();
    let doubled = wsr_null_pmf_doubled(&ranks);
    // doubled index 2k -> R+ = k
    let m = n * (n + 1) / 2;
    let mut pmf = vec![0.0f64; m + 1];
    for k in 0..=m {
        pmf[k] = doubled[2 * k];
    }
    pmf
}

/// Distribution-free CI for the median paired difference, from inverting the exact
/// signed-rank test (Tukey / Hollander & Wolfe §3.2): with `k = qsignrank(alpha/2, n)`
/// the interval is `[W_(k), W_(m-k+1)]` over the sorted Walsh averages,
/// `m = n(n+1)/2`. Reproduces R's `wilcox.test(x, conf.int=TRUE)`.
pub fn hodges_lehmann_paired_ci(d: &[f64], alpha: f64) -> ShiftCi {
    let n = d.len();
    if n > EXACT_WSR_MAX_N {
        return hodges_lehmann_paired_ci_normal(d, alpha);
    }
    let w = walsh_averages(d);
    let m = w.len();
    let pmf = wsr_null_pmf_untied(n);

    let absd: Vec<f64> = d.iter().map(|v| v.abs()).collect();
    let (_, t) = mid_ranks(&absd);
    let exact = !t.iter().any(|&g| g > 1) && !d.iter().any(|v| *v == 0.0);

    let (k, cum) = ci_trim(&pmf, m, alpha);
    let est = if m % 2 == 1 {
        w[m / 2]
    } else {
        0.5 * (w[m / 2 - 1] + w[m / 2])
    };
    ShiftCi {
        estimate: est,
        lo: w[k - 1],
        hi: w[m - k],
        achieved_level: 1.0 - 2.0 * cum,
        k: k - 1,
        exact,
    }
}

// ---------------------------------------------------------------------------
// 6. Exact / Monte-Carlo permutation test on an arbitrary two-sample statistic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct PermResult {
    pub observed: f64,
    pub p_two_sided: f64,
    /// Number of permutations actually used (exhaustive count, or the MC budget).
    pub n_perm: u64,
    pub exhaustive: bool,
}

/// Difference of medians, `median(a) - median(b)`. The natural statistic for the
/// "weak input"/actuation-count variant.
pub fn diff_of_medians(a: &[f64], b: &[f64]) -> f64 {
    median(a) - median(b)
}

/// Cap on the number of exhaustive partitions before falling back to Monte Carlo.
pub const PERM_EXHAUSTIVE_MAX: u64 = 5_000_000;

fn n_choose_k(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut r: u128 = 1;
    for i in 0..k {
        r = r * (n - i) as u128 / (i + 1) as u128;
        if r > u64::MAX as u128 {
            return u64::MAX;
        }
    }
    r as u64
}

struct XorShift64(u64);

impl XorShift64 {
    /// SplitMix64 finalisation of the caller's seed, so nearby seeds give
    /// unrelated streams and no seed maps onto another.
    fn seeded(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        XorShift64(if z == 0 { 0x9E37_79B9_7F4A_7C15 } else { z })
    }
}
impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// Two-sided permutation test for a shift in `stat(a, b)` under the null that the
/// pooled labels are exchangeable.
///
/// p-value convention matches `scipy.stats.permutation_test(..., permutation_type='independent',
/// alternative='two-sided')`:
///   * exhaustive: `p_less = #{T <= t_obs + g}/M`, `p_greater = #{T >= t_obs - g}/M`,
///     `p = min(2*min(p_less,p_greater), 1)`, with `g = 100*eps*|t_obs|`;
///   * Monte-Carlo: both numerator and denominator get `+1` (the conservative
///     Phipson-Smyth adjustment scipy uses).
pub fn perm_test<F: Fn(&[f64], &[f64]) -> f64>(
    a: &[f64],
    b: &[f64],
    stat: F,
    mc_resamples: u64,
    seed: u64,
) -> PermResult {
    let n1 = a.len();
    let n2 = b.len();
    let n = n1 + n2;
    let mut pooled = a.to_vec();
    pooled.extend_from_slice(b);
    let obs = stat(a, b);
    let gamma = 100.0 * f64::EPSILON * obs.abs();

    let total = n_choose_k(n as u64, n1 as u64);
    let exhaustive = total <= PERM_EXHAUSTIVE_MAX;

    let mut n_le = 0u64;
    let mut n_ge = 0u64;
    let mut m = 0u64;
    let mut buf_a = vec![0.0f64; n1];
    let mut buf_b = vec![0.0f64; n2];

    if exhaustive {
        // iterate over all C(n, n1) index subsets in lexicographic order
        let mut idx: Vec<usize> = (0..n1).collect();
        let mut mark = vec![false; n];
        loop {
            for v in mark.iter_mut() {
                *v = false;
            }
            for &i in &idx {
                mark[i] = true;
            }
            let (mut ia, mut ib) = (0usize, 0usize);
            for i in 0..n {
                if mark[i] {
                    buf_a[ia] = pooled[i];
                    ia += 1;
                } else {
                    buf_b[ib] = pooled[i];
                    ib += 1;
                }
            }
            let t = stat(&buf_a, &buf_b);
            if t <= obs + gamma {
                n_le += 1;
            }
            if t >= obs - gamma {
                n_ge += 1;
            }
            m += 1;
            // advance to the next combination
            let mut i = n1;
            let mut advanced = false;
            while i > 0 {
                i -= 1;
                if idx[i] != i + n - n1 {
                    idx[i] += 1;
                    for j in (i + 1)..n1 {
                        idx[j] = idx[j - 1] + 1;
                    }
                    advanced = true;
                    break;
                }
            }
            if !advanced {
                break;
            }
        }
        let p_less = n_le as f64 / m as f64;
        let p_greater = n_ge as f64 / m as f64;
        PermResult {
            observed: obs,
            p_two_sided: (2.0 * p_less.min(p_greater)).clamp(0.0, 1.0),
            n_perm: m,
            exhaustive: true,
        }
    } else {
        // Mixed, not `seed | 1`: forcing the low bit maps every even seed onto
        // its odd successor, so half the seed space is unreachable and two
        // "different" seeds silently give identical resamples.
        let mut rng = XorShift64::seeded(seed);
        let mut work = pooled.clone();
        for _ in 0..mc_resamples {
            // Fisher-Yates
            for i in (1..n).rev() {
                let j = rng.below((i + 1) as u64) as usize;
                work.swap(i, j);
            }
            buf_a.copy_from_slice(&work[..n1]);
            buf_b.copy_from_slice(&work[n1..]);
            let t = stat(&buf_a, &buf_b);
            if t <= obs + gamma {
                n_le += 1;
            }
            if t >= obs - gamma {
                n_ge += 1;
            }
        }
        let p_less = (n_le as f64 + 1.0) / (mc_resamples as f64 + 1.0);
        let p_greater = (n_ge as f64 + 1.0) / (mc_resamples as f64 + 1.0);
        PermResult {
            observed: obs,
            p_two_sided: (2.0 * p_less.min(p_greater)).clamp(0.0, 1.0),
            n_perm: mc_resamples,
            exhaustive: false,
        }
    }
}

/// Paired (sign-flip) permutation test on the differences `d`, for an arbitrary
/// statistic of `d`. Exhaustive when `n <= 22` (4.2M sign patterns).
pub fn sign_flip_test<F: Fn(&[f64]) -> f64>(
    d: &[f64],
    stat: F,
    mc_resamples: u64,
    seed: u64,
) -> PermResult {
    let n = d.len();
    let obs = stat(d);
    let gamma = 100.0 * f64::EPSILON * obs.abs();
    let total: u64 = if n < 63 { 1u64 << n } else { u64::MAX };
    let exhaustive = total <= PERM_EXHAUSTIVE_MAX;
    let mut n_le = 0u64;
    let mut n_ge = 0u64;
    let mut buf = vec![0.0f64; n];

    if exhaustive {
        for mask in 0..total {
            for i in 0..n {
                buf[i] = if mask >> i & 1 == 1 { -d[i] } else { d[i] };
            }
            let t = stat(&buf);
            if t <= obs + gamma {
                n_le += 1;
            }
            if t >= obs - gamma {
                n_ge += 1;
            }
        }
        let pl = n_le as f64 / total as f64;
        let pg = n_ge as f64 / total as f64;
        PermResult {
            observed: obs,
            p_two_sided: (2.0 * pl.min(pg)).clamp(0.0, 1.0),
            n_perm: total,
            exhaustive: true,
        }
    } else {
        // Mixed, not `seed | 1`: forcing the low bit maps every even seed onto
        // its odd successor, so half the seed space is unreachable and two
        // "different" seeds silently give identical resamples.
        let mut rng = XorShift64::seeded(seed);
        for _ in 0..mc_resamples {
            // One word per 64 observations. Drawing a single word and reusing
            // it made positions i and i+64 always flip together, so only 2^64
            // of the 2^n sign patterns were reachable.
            let mut word = rng.next_u64();
            for i in 0..n {
                if i > 0 && i % 64 == 0 {
                    word = rng.next_u64();
                }
                buf[i] = if (word >> (i % 64)) & 1 == 1 { -d[i] } else { d[i] };
            }
            let t = stat(&buf);
            if t <= obs + gamma {
                n_le += 1;
            }
            if t >= obs - gamma {
                n_ge += 1;
            }
        }
        let pl = (n_le as f64 + 1.0) / (mc_resamples as f64 + 1.0);
        let pg = (n_ge as f64 + 1.0) / (mc_resamples as f64 + 1.0);
        PermResult {
            observed: obs,
            p_two_sided: (2.0 * pl.min(pg)).clamp(0.0, 1.0),
            n_perm: mc_resamples,
            exhaustive: false,
        }
    }
}

// ---------------------------------------------------------------------------
// 7. Top-level report for the UI
// ---------------------------------------------------------------------------

/// Which analysis the UI shows as *the* answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primary {
    /// Wilcoxon signed-rank on the within-block differences. Default for an
    /// interleaved A/B run, because pairing removes session drift/fatigue.
    Paired,
    /// Mann-Whitney U. Used when the run was not strictly interleaved, or when
    /// pairs are missing/unequal in number.
    Unpaired,
}

#[derive(Debug, Clone)]
pub struct AbReport {
    pub primary: Primary,
    pub summary_a: RobustSummary,
    pub summary_b: RobustSummary,
    /// Primary p-value: from `paired` when `primary == Paired`, else from `unpaired`.
    pub p_primary: f64,
    pub paired: Option<WsrResult>,
    pub paired_shift: Option<ShiftCi>,
    pub unpaired: MwuResult,
    pub unpaired_shift: ShiftCi,
    /// Set when the primary test's exact branch could not be used.
    pub notes: Vec<String>,
}

/// Compute the whole report. `a` and `b` must be in trial order. If they are equal
/// in length and the run was interleaved, the paired analysis is used as primary.
pub fn analyse(a: &[f64], b: &[f64], interleaved: bool, alpha: f64) -> AbReport {
    let mut notes = Vec::new();
    let unpaired = mann_whitney_u(a, b, MwuMethod::Auto);
    let unpaired_shift = hodges_lehmann_shift_ci(a, b, alpha);
    if !unpaired_shift.exact {
        notes.push(
            "Ties in the pooled sample: the unpaired shift interval is conservative, \
             not exact."
                .to_string(),
        );
    }

    let (primary, paired, paired_shift, p_primary) = if interleaved && a.len() == b.len() {
        let d: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
        let n_zero = d.iter().filter(|v| **v == 0.0).count();
        // Pratt keeps the zero pairs in the ranking; with discrete actuation counts
        // there are usually several, and discarding them (Wilcox) is anti-conservative.
        let zm = if n_zero > 0 {
            ZeroMethod::Pratt
        } else {
            ZeroMethod::Wilcox
        };
        if n_zero > 0 {
            notes.push(format!(
                "{n_zero} of {} pairs tied exactly; using Pratt's zero handling.",
                d.len()
            ));
        }
        let w = wilcoxon_signed_rank(&d, zm, false, WsrMethod::Auto);
        let ci = hodges_lehmann_paired_ci(&d, alpha);
        let p = w.p_two_sided;
        (Primary::Paired, Some(w), Some(ci), p)
    } else {
        if interleaved {
            notes.push("Unequal trial counts: falling back to the unpaired test.".to_string());
        }
        (
            Primary::Unpaired,
            None,
            None,
            unpaired.p_two_sided,
        )
    };

    AbReport {
        primary,
        summary_a: robust_summary(a),
        summary_b: robust_summary(b),
        p_primary,
        paired,
        paired_shift,
        unpaired,
        unpaired_shift,
        notes,
    }
}

/// Fixed text the UI must show next to the primary result. One run, one primary
/// test; anything else needs a correction and a re-plan, not another peek.
pub const MULTIPLICITY_NOTICE: &str = concat!(
    "One run, one primary test. This p-value is only valid if the number of trials and ",
    "the primary test were both fixed before the run started. Do not add pairs and re-check ",
    "until it drops below 0.05: measured on this design (Wilcoxon signed-rank, null data, ",
    "peeking after every pair from n=6 to n=20), that turns a 5% false-positive rate into ",
    "16%; peeking just three times still gives 10%. The secondary tests shown below the ",
    "primary are descriptive, not confirmations - agreeing p-values from the same data are ",
    "not independent evidence. If you genuinely want several metrics, pre-declare one as ",
    "primary and treat the rest as exploratory, or apply a Holm correction across all of them."
);

/// Measured false-positive rates for the peeking scenarios quoted in
/// [`MULTIPLICITY_NOTICE`] (4000 null simulations each, n = 6..20 pairs,
/// Wilcoxon signed-rank at alpha = 0.05):
///   fixed n=20, single test .......... 0.049
///   fixed n=6,  single test .......... 0.032  (discreteness makes it conservative)
///   peek after every pair, 6..20 ..... 0.162
///   peek every 2nd pair (8 looks) .... 0.142
///   peek 3 times (n = 6, 12, 20) ..... 0.097
pub const PEEKING_TYPE_I: [(&str, f64); 5] = [
    ("fixed n=20", 0.0493),
    ("fixed n=6", 0.0315),
    ("peek every pair 6..20", 0.1615),
    ("peek every 2nd pair", 0.1417),
    ("peek 3 times", 0.0968),
];

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midranks_basic() {
        let (r, t) = mid_ranks(&[1.0, 2.0, 2.0, 3.0]);
        assert_eq!(r, vec![1.0, 2.5, 2.5, 4.0]);
        assert_eq!(t, vec![1, 2, 1]);
    }

    #[test]
    fn quantile_matches_numpy_type7() {
        let x = [1.0, 2.0, 3.0, 4.0];
        assert!((quantile_type7(&x, 0.25) - 1.75).abs() < 1e-12);
        assert!((quantile_type7(&x, 0.5) - 2.5).abs() < 1e-12);
        assert!((quantile_type7(&x, 0.75) - 3.25).abs() < 1e-12);
    }

    #[test]
    fn mwu_null_sums_to_one() {
        for n1 in 1..8 {
            for n2 in 1..8 {
                let p = mwu_null_pmf(n1, n2);
                let s: f64 = p.iter().sum();
                assert!((s - 1.0).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn rank_biserial_sign() {
        // A strictly above B => r = +1, prob_superiority = 1
        let r = mann_whitney_u(&[5.0, 6.0, 7.0], &[1.0, 2.0, 3.0], MwuMethod::Auto);
        assert!((r.rank_biserial - 1.0).abs() < 1e-12);
        assert!((r.prob_superiority - 1.0).abs() < 1e-12);
    }
}


/// Normal-approximation shift interval, used above the exact enumeration cap.
///
/// The trim index comes from the normal quantile of the Mann-Whitney null
/// rather than from its enumerated distribution, so the coverage is
/// approximate. `exact` is false and `achieved_level` reports the nominal
/// level, which the interface must show as approximate.
fn hodges_lehmann_shift_ci_normal(a: &[f64], b: &[f64], alpha: f64) -> ShiftCi {
    let n1 = a.len() as f64;
    let n2 = b.len() as f64;
    let d = pairwise_diffs(a, b);
    let m = d.len();
    let est = if m == 0 {
        f64::NAN
    } else if m % 2 == 1 {
        d[m / 2]
    } else {
        0.5 * (d[m / 2 - 1] + d[m / 2])
    };
    if m == 0 {
        return ShiftCi { estimate: est, lo: f64::NAN, hi: f64::NAN,
                         achieved_level: 0.0, k: 0, exact: false };
    }
    let mean = n1 * n2 / 2.0;
    let sd = (n1 * n2 * (n1 + n2 + 1.0) / 12.0).sqrt();
    let z = norm_quantile(alpha / 2.0);
    // k is the number of order statistics trimmed from each tail.
    let k = ((mean + z * sd).floor().max(0.0)) as usize;
    let k = k.min(m.saturating_sub(1) / 2);
    ShiftCi {
        estimate: est,
        lo: d[k],
        hi: d[m - 1 - k],
        achieved_level: 1.0 - alpha,
        k,
        exact: false,
    }
}

/// Normal-approximation paired interval, used above the exact enumeration cap.
fn hodges_lehmann_paired_ci_normal(d: &[f64], alpha: f64) -> ShiftCi {
    let n = d.len() as f64;
    let w = walsh_averages(d);
    let m = w.len();
    let est = if m == 0 {
        f64::NAN
    } else if m % 2 == 1 {
        w[m / 2]
    } else {
        0.5 * (w[m / 2 - 1] + w[m / 2])
    };
    if m == 0 {
        return ShiftCi { estimate: est, lo: f64::NAN, hi: f64::NAN,
                         achieved_level: 0.0, k: 0, exact: false };
    }
    let mean = n * (n + 1.0) / 4.0;
    let sd = (n * (n + 1.0) * (2.0 * n + 1.0) / 24.0).sqrt();
    let z = norm_quantile(alpha / 2.0);
    let k = ((mean + z * sd).floor().max(0.0)) as usize;
    let k = k.min(m.saturating_sub(1) / 2);
    ShiftCi {
        estimate: est,
        lo: w[k],
        hi: w[m - 1 - k],
        achieved_level: 1.0 - alpha,
        k,
        exact: false,
    }
}

/// Inverse standard normal CDF, Acklam's rational approximation refined by one
/// Halley step against `norm_sf`. Accurate to better than 1e-12 over the range
/// a confidence interval needs.
pub fn norm_quantile(p: f64) -> f64 {
    if !(p > 0.0 && p < 1.0) {
        return f64::NAN;
    }
    const A: [f64; 6] = [
        -3.969683028665376e+01, 2.209460984245205e+02, -2.759285104469687e+02,
        1.383577518672690e+02, -3.066479806614716e+01, 2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01, 1.615858368580409e+02, -1.556989798598866e+02,
        6.680131188771972e+01, -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e+00,
        -2.549732539343734e+00, 4.374664141464968e+00, 2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    const P_LOW: f64 = 0.02425;
    let x = if p < P_LOW {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - P_LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    // One Halley refinement.
    let e = norm_sf(x) - (1.0 - p);
    let u = e * (2.0 * std::f64::consts::PI).sqrt() * (x * x / 2.0).exp();
    x - u / (1.0 + x * u / 2.0)
}

#[cfg(test)]
mod regression_tests {
    //! Guards for defects found by review of this module before it was adopted.
    //! The numeric agreement with scipy is checked separately by
    //! `scripts/verify_stats.py`, which compares nineteen cases against
    //! scipy and brute force; these cover the things a reference cannot catch.

    use super::*;

    #[test]
    fn different_seeds_give_different_resamples() {
        // `XorShift64(seed | 1)` mapped every even seed onto its odd successor,
        // so half the seed space was unreachable and two seeds a user might
        // reasonably pick gave byte-identical results.
        let a: Vec<f64> = (0..13).map(|i| i as f64).collect();
        let b: Vec<f64> = (0..13).map(|i| i as f64 + 0.7).collect();
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..8u64 {
            let r = perm_test(&a, &b, diff_of_medians, 4_000, seed);
            seen.insert(r.p_two_sided.to_bits());
        }
        assert!(
            seen.len() >= 6,
            "only {} distinct results across 8 seeds; seeds are aliasing",
            seen.len()
        );
    }

    #[test]
    fn sign_flips_are_independent_beyond_sixty_four_observations() {
        // One 64-bit word per resample meant observation i and i+64 always
        // flipped together, so only a vanishing fraction of the sign patterns
        // were reachable and the p-value was biased for any run over 64 pairs.
        let n = 70;
        let d: Vec<f64> = (0..n).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        // Records which combination of signs positions 0 and 64 took.
        let seen = std::cell::RefCell::new(std::collections::BTreeSet::new());
        let stat = |x: &[f64]| {
            let s0 = x[0] > 0.0;
            let s64 = x[64] > 0.0;
            seen.borrow_mut().insert((s0, s64));
            x.iter().sum::<f64>()
        };
        let _ = sign_flip_test(&d, stat, 3_000, 99);
        assert_eq!(
            seen.borrow().len(),
            4,
            "positions 0 and 64 only reached {:?}; they are not flipping independently",
            seen.borrow()
        );
    }

    #[test]
    fn confidence_intervals_do_not_panic_above_the_exact_cap() {
        // These asserted, so an ordinary long run took down the interface.
        let a: Vec<f64> = (0..40).map(|i| i as f64 * 0.3).collect();
        let b: Vec<f64> = (0..40).map(|i| i as f64 * 0.3 + 1.0).collect();
        let u = hodges_lehmann_shift_ci(&a, &b, 0.05);
        assert!(!u.exact, "above the cap the interval cannot be exact");
        assert!(u.lo.is_finite() && u.hi.is_finite());
        assert!(u.lo <= u.estimate && u.estimate <= u.hi);

        let d: Vec<f64> = (0..60).map(|i| (i % 7) as f64 - 3.0).collect();
        let p = hodges_lehmann_paired_ci(&d, 0.05);
        assert!(!p.exact);
        assert!(p.lo.is_finite() && p.hi.is_finite());
        assert!(p.lo <= p.estimate && p.estimate <= p.hi);
    }

    #[test]
    fn the_top_level_report_survives_a_long_run() {
        let a: Vec<f64> = (0..80).map(|i| 10.0 + (i % 5) as f64 * 0.1).collect();
        let b: Vec<f64> = (0..80).map(|i| 10.4 + (i % 5) as f64 * 0.1).collect();
        let r = analyse(&a, &b, true, 0.05);
        assert!(r.p_primary.is_finite());
        assert!(r.paired.is_some());
    }

    #[test]
    fn requesting_the_exact_branch_above_its_cap_degrades_rather_than_aborting() {
        let a: Vec<f64> = (0..40).map(|i| i as f64).collect();
        let b: Vec<f64> = (0..40).map(|i| i as f64 + 2.0).collect();
        let r = mann_whitney_u(&a, &b, MwuMethod::Exact);
        assert!(!r.exact);
        assert!(r.p_two_sided.is_finite());
        let d: Vec<f64> = (0..60).map(|i| ((i % 9) as f64) - 4.0).collect();
        let w = wilcoxon_signed_rank(&d, ZeroMethod::Pratt, false, WsrMethod::Exact);
        assert!(!w.exact);
        assert!(w.p_two_sided.is_finite());
    }

    #[test]
    fn degenerate_inputs_are_representable_so_the_interface_can_guard_them() {
        // Every pair identical: there is no effect to size, and the effect size
        // is genuinely undefined rather than zero.
        let x = vec![1.0, 2.0, 3.0, 4.0];
        let r = analyse(&x, &x, true, 0.05);
        assert!(
            r.paired.as_ref().unwrap().rank_biserial.is_nan(),
            "an undefined effect size must be NaN, not a misleading zero"
        );
        // A single pair cannot support any interval at all.
        let one = analyse(&[5.0], &[3.0], true, 0.05);
        let ci = one.paired_shift.as_ref().unwrap();
        assert_eq!(
            ci.achieved_level, 0.0,
            "a one-pair interval has no coverage and must say so"
        );
    }
}
