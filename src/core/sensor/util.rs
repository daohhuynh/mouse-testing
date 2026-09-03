//! Dependency-free statistics + a radix-2 FFT. Used by several detectors.

// The full API of this module is kept, not just the part the interface reads
// today. These are self-contained numerical routines checked as a whole against
// an outside reference, and trimming them to the current call sites would make
// that check harder to repeat than the unused functions are worth.
#![allow(dead_code)]

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { return 0.0; }
    v.iter().sum::<f64>() / v.len() as f64
}

pub fn variance(v: &[f64]) -> f64 {
    if v.len() < 2 { return 0.0; }
    let m = mean(v);
    v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64
}

pub fn stddev(v: &[f64]) -> f64 { variance(v).sqrt() }

pub fn rms(v: &[f64]) -> f64 {
    if v.is_empty() { return 0.0; }
    (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt()
}

/// Linear-interpolated percentile, p in [0,1]. Sorts a copy.
pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() { return f64::NAN; }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = p.clamp(0.0, 1.0) * (s.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi { s[lo] } else { s[lo] + (idx - lo as f64) * (s[hi] - s[lo]) }
}

pub fn median(v: &[f64]) -> f64 { percentile(v, 0.5) }

/// Median absolute deviation, scaled to be a consistent estimator of sigma for
/// Gaussian data (factor 1.4826).
pub fn mad_sigma(v: &[f64]) -> f64 {
    if v.len() < 2 { return 0.0; }
    let m = median(v);
    let dev: Vec<f64> = v.iter().map(|x| (x - m).abs()).collect();
    1.4826 * median(&dev)
}

/// Sample autocorrelation at lag `k` of an already zero-meaned-ish series.
/// Uses the biased (divide-by-N) estimator, which is what you want for spectral
/// consistency and never exceeds 1.
pub fn autocorr(v: &[f64], k: usize) -> f64 {
    if v.len() <= k + 1 { return 0.0; }
    let m = mean(v);
    let mut num = 0.0;
    let mut den = 0.0;
    for i in 0..v.len() {
        let a = v[i] - m;
        den += a * a;
        if i + k < v.len() { num += a * (v[i + k] - m); }
    }
    if den <= 0.0 { 0.0 } else { num / den }
}

/// Centered moving average with window `w` (forced odd). Edges use a shrinking window.
/// Subtracting this from the signal is our canonical high-pass.
pub fn movavg(v: &[f64], w: usize) -> Vec<f64> {
    let w = if w % 2 == 0 { w + 1 } else { w };
    let h = w / 2;
    let n = v.len();
    // prefix sums for O(n)
    let mut pre = vec![0.0f64; n + 1];
    for i in 0..n { pre[i + 1] = pre[i] + v[i]; }
    (0..n).map(|i| {
        let lo = i.saturating_sub(h);
        let hi = (i + h + 1).min(n);
        (pre[hi] - pre[lo]) / (hi - lo) as f64
    }).collect()
}

/// v - movavg(v, w): a zero-phase high-pass. Returns (residual, effective_window).
pub fn highpass(v: &[f64], w: usize) -> (Vec<f64>, usize) {
    let w = if w % 2 == 0 { w + 1 } else { w };
    let ma = movavg(v, w);
    (v.iter().zip(ma.iter()).map(|(a, b)| a - b).collect(), w)
}

/// v - movavg(movavg(v, w), w): a CASCADED (triangular-kernel) zero-phase high-pass.
///
/// USE THIS, NOT `highpass`, WHENEVER THE THING YOU ARE MEASURING IS A CORRELATION.
/// A single moving average has first sidelobes only -13 dB down, so an 8-12 Hz
/// physiological tremor leaks straight through a 20 Hz corner. A leaked 10 Hz sinusoid
/// sampled at 1 kHz has a lag-1 autocorrelation of cos(2*pi*10/1000) = 0.998, so even a
/// tiny leak drags rho1 upward and makes a perfectly clean mouse look filtered. Measured:
/// clean rho1 was 0.272 with the single-stage filter (a Warn), and the cascade fixes it.
/// The triangular kernel's sidelobes are -26 dB, which is enough.
pub fn highpass2(v: &[f64], w: usize) -> (Vec<f64>, usize) {
    let w = if w % 2 == 0 { w + 1 } else { w };
    let ma = movavg(&movavg(v, w), w);
    (v.iter().zip(ma.iter()).map(|(a, b)| a - b).collect(), w)
}

/// Ordinary least squares y = a + b*x. Returns (a, b, r2).
pub fn ols(x: &[f64], y: &[f64]) -> (f64, f64, f64) {
    let n = x.len().min(y.len());
    if n < 2 { return (0.0, 0.0, 0.0); }
    let mx = mean(&x[..n]);
    let my = mean(&y[..n]);
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for i in 0..n {
        let dx = x[i] - mx;
        let dy = y[i] - my;
        sxy += dx * dy; sxx += dx * dx; syy += dy * dy;
    }
    if sxx <= 0.0 { return (my, 0.0, 0.0); }
    let b = sxy / sxx;
    let a = my - b * mx;
    let r2 = if syy <= 0.0 { 0.0 } else { (sxy * sxy) / (sxx * syy) };
    (a, b, r2)
}

/// Total-least-squares (PCA) line fit through 2-D points.
/// Returns (centroid, unit direction of the principal axis, RMS perpendicular residual).
pub fn tls_line(pts: &[(f64, f64)]) -> ((f64, f64), (f64, f64), f64) {
    let n = pts.len();
    if n < 2 { return ((0.0, 0.0), (1.0, 0.0), 0.0); }
    let cx = pts.iter().map(|p| p.0).sum::<f64>() / n as f64;
    let cy = pts.iter().map(|p| p.1).sum::<f64>() / n as f64;
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for &(x, y) in pts {
        let a = x - cx; let b = y - cy;
        sxx += a * a; syy += b * b; sxy += a * b;
    }
    // principal eigenvector of [[sxx,sxy],[sxy,syy]]
    let tr = sxx + syy;
    let det = sxx * syy - sxy * sxy;
    let disc = ((tr * tr / 4.0) - det).max(0.0).sqrt();
    let l1 = tr / 2.0 + disc;
    let (mut ux, mut uy) = if sxy.abs() > 1e-12 {
        (l1 - syy, sxy)
    } else if sxx >= syy { (1.0, 0.0) } else { (0.0, 1.0) };
    let nrm = (ux * ux + uy * uy).sqrt();
    if nrm > 0.0 { ux /= nrm; uy /= nrm; } else { ux = 1.0; uy = 0.0; }
    // perpendicular residuals
    let (px, py) = (-uy, ux);
    let mut ss = 0.0;
    for &(x, y) in pts {
        let d = (x - cx) * px + (y - cy) * py;
        ss += d * d;
    }
    ((cx, cy), (ux, uy), (ss / n as f64).sqrt())
}

// ---------------- FFT ----------------

/// In-place iterative radix-2 Cooley-Tukey FFT. `re`/`im` length must be a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(n.is_power_of_two(), "fft length must be a power of two");
    assert_eq!(n, im.len());
    // bit-reversal permutation
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 { j ^= bit; bit >>= 1; }
        j |= bit;
        if i < j { re.swap(i, j); im.swap(i, j); }
    }
    let mut len = 2usize;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0usize;
        while i < n {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let a = i + k;
                let b = i + k + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr; im[b] = im[a] - ti;
                re[a] += tr; im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
            i += len;
        }
        len <<= 1;
    }
}

/// One-sided power spectral density estimate via Welch's method with a Hann window,
/// 50% overlap. Returns (freqs_hz, psd) with psd normalised so that
/// sum(psd)*df == variance of the input (approximately).
pub fn welch_psd(x: &[f64], fs: f64, seg: usize) -> (Vec<f64>, Vec<f64>) {
    let seg = seg.next_power_of_two();
    if x.len() < seg { return (vec![], vec![]); }
    let hop = seg / 2;
    let win: Vec<f64> = (0..seg)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / seg as f64).cos())
        .collect();
    let wpow: f64 = win.iter().map(|w| w * w).sum::<f64>();
    let nbins = seg / 2 + 1;
    let mut acc = vec![0.0f64; nbins];
    let mut nseg = 0usize;
    let mut s = 0usize;
    while s + seg <= x.len() {
        let m = mean(&x[s..s + seg]);
        let mut re: Vec<f64> = (0..seg).map(|i| (x[s + i] - m) * win[i]).collect();
        let mut im = vec![0.0f64; seg];
        fft(&mut re, &mut im);
        for k in 0..nbins {
            let p = re[k] * re[k] + im[k] * im[k];
            // one-sided: double interior bins
            let scale = if k == 0 || k == seg / 2 { 1.0 } else { 2.0 };
            acc[k] += scale * p / (fs * wpow);
        }
        nseg += 1;
        s += hop;
    }
    if nseg == 0 { return (vec![], vec![]); }
    for a in acc.iter_mut() { *a /= nseg as f64; }
    let df = fs / seg as f64;
    ((0..nbins).map(|k| k as f64 * df).collect(), acc)
}

/// Integrate a PSD over [f_lo, f_hi].
pub fn band_power(freqs: &[f64], psd: &[f64], f_lo: f64, f_hi: f64) -> f64 {
    if freqs.len() < 2 { return 0.0; }
    let df = freqs[1] - freqs[0];
    freqs.iter().zip(psd.iter())
        .filter(|(f, _)| **f >= f_lo && **f <= f_hi)
        .map(|(_, p)| *p * df)
        .sum()
}

/// Lag-1 autocorrelation that the cascaded-MA high-pass `highpass2` imposes on WHITE
/// input, computed exactly from its FIR kernel. Subtract this from a measured rho1 to get
/// an estimator whose null value is 0.
///
/// The filter is g = delta - h, where h is two length-w boxcars convolved, i.e. the
/// triangular kernel h[i] = (w - |i|)/w^2 for |i| < w. For white input the residual's
/// autocovariance at lag k is sigma^2 * (g corr g)(k), so rho1 = Rg(1)/Rg(0) exactly.
/// Guessing this offset (we tried -1/w and +2/w) is what produced a clean-mouse rho1 of
/// +0.32 and a 100% false-FAIL rate; do not guess it.
pub fn highpass2_white_rho1(w: usize) -> f64 {
    let w = if w % 2 == 0 { w + 1 } else { w };
    let wf = w as f64;
    let n = 2 * w - 1;
    let c = w - 1; // index of lag 0 in the kernel
    let mut g = vec![0.0f64; n];
    for i in 0..n {
        let lag = i as isize - c as isize;
        let tri = (wf - lag.abs() as f64) / (wf * wf);
        g[i] = -tri;
    }
    g[c] += 1.0;
    let r0: f64 = g.iter().map(|x| x * x).sum();
    let r1: f64 = (0..n - 1).map(|i| g[i] * g[i + 1]).sum();
    if r0 > 0.0 { r1 / r0 } else { 0.0 }
}

/// Two-sided normal tail probability, from a rational approximation to erfc
/// (Numerical Recipes `erfcc`, |error| < 1.2e-7 relative).
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 2.0 / (2.0 + z);
    let ty = 4.0 * t - 2.0;
    const C: [f64; 10] = [
        -1.3026537197817094, 6.4196979235649026e-1, 1.9476473204185836e-2,
        -9.561514786808631e-3, -9.46595344482036e-4, 3.66839497852761e-4,
        4.2523324806907e-5, -2.0278578112534e-5, -1.624290004647e-6, 1.303655835580e-6,
    ];
    let (mut d, mut dd) = (0.0f64, 0.0f64);
    for j in (1..C.len()).rev() {
        let tmp = d;
        d = ty * d - dd + C[j];
        dd = tmp;
    }
    let ans = t * (-z * z + 0.5 * (C[0] + ty * d) - dd).exp();
    if x >= 0.0 { ans } else { 2.0 - ans }
}

/// P(|Z| > |z|) for standard normal Z.
pub fn two_sided_p(z: f64) -> f64 { erfc(z.abs() / std::f64::consts::SQRT_2) }
