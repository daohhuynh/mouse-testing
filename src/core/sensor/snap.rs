//! Detector 3: angle snapping / path (prediction) correction.
//!
//! PROTOCOL: user draws ONE hand-drawn line, left-to-right, ~4-8 inches of mouse travel,
//! at a normal aiming speed (not creeping). Repeat 5x.
//!
//! THE HARD PART is that the obvious statistic -- "fraction of reports with dy == 0 while
//! dx != 0" -- is useless at high poll rates. At 8 kHz / 1600 CPI / 10 IPS, the true
//! per-report step is 2.0 counts along travel; a 5-degree line has a true dy of 0.175
//! counts/report, so a PERFECT device emits dy == 0 in ~82% of reports purely from
//! integer quantisation. So:
//!   (1) we compare the observed dy==0 fraction to the fraction PREDICTED by the observed
//!       mean |dy| under a residual-accumulator quantiser (P(dy=0) = 1 - E|dy| for
//!       E|dy| < 1), and only the EXCESS counts; and
//!   (2) the primary statistics are computed on TIME-BINNED deltas, with the bin chosen
//!       so each bin carries >= `min_bin_counts` counts of travel, which removes
//!       quantisation from the geometry entirely.
//!
//! THE PRIMARY STATISTIC is HIGH-FREQUENCY ANISOTROPY.
//! Sensor noise is isotropic: the sensor does not know which way the hand is going, so
//! its per-report error has the same variance along the stroke as across it. Angle
//! snapping projects every increment onto one direction, which deletes the ACROSS
//! component of that noise while leaving the ALONG component untouched. So
//!     hf_aniso = RMS(highpass(perp increments)) / RMS(highpass(along increments))
//! is near 1.0 for an honest digitiser and drops well below 1 for a snapper. The
//! along-axis residual is the device measuring its OWN noise level for us, which is
//! what keeps this free of a hand-tuned constant.
//!
//! HOW NEAR 1.0, HONESTLY. It is tempting to call this calibration-free and stop
//! there, and the first version of this comment did. The measured null actually
//! moves with sensor noise and poll rate: 1.539 at 1 kHz with a quiet 0.35
//! counts/report sensor, 1.133 at 8 kHz, 1.000 at 2.0 counts, 0.971 at 4.0. All of
//! that error is in the safe direction, so a clean mouse never reaches the 0.55
//! FAIL band. The unsafe case is the other one: a genuinely snapping mouse whose
//! sensor is too quiet for the statistic to have anything to measure reads ~1.05
//! and passes. That is why the along-axis residual RMS is checked first and the
//! test reports itself NOT APPLICABLE below `aniso_min_along_rms` rather than
//! passing on a measurement it could not make.
//!
//! A NOTE ON THE QUANTISATION FLOOR, because the obvious version of this argument is
//! WRONG and the simulator caught it. It is tempting to say: a residual-accumulator
//! quantiser has a perpendicular position error uniform on (-0.5,+0.5), so the bin-to-bin
//! perpendicular increment must have sd >= sqrt(2/12) = 0.408 counts, and anything
//! quieter is synthetic. That only holds if the firmware snaps AFTER quantising (integer
//! deltas placed on an exact line). If it snaps BEFORE quantising -- which is the normal
//! implementation, since the filter runs on the sensor's fractional motion estimate --
//! the quantiser is still downstream and still injects its full 0.408, so `hf_perp_ratio`
//! stays near 1 and detects NOTHING. Measured: hf_perp_ratio was 1.72 on a fully snapped
//! stream vs 2.05 clean. It is retained as a secondary statistic only, because it does
//! catch the post-quantisation variant.
//!
//! THE SLOW-CAREFUL CONFOUND: a slow, careful stroke really is straight in the
//! low-frequency band. It is NOT quiet in the high-frequency band -- physiological tremor
//! (8-12 Hz) and sensor noise are still there. So the confound guard is a band split:
//! `hf_perp_ratio` measures perpendicular energy ABOVE ~15 Hz relative to the
//! quantisation floor. Human voluntary motor control is band-limited to about 5 Hz and
//! tremor lives at 8-12 Hz, so nothing a human does can *remove* HF perpendicular energy.
//! Only firmware can. We additionally refuse to judge strokes that are too slow or too
//! short (Inconclusive, not Pass).

use super::seg::{bin_deltas, dominant_span, MotionGate};
use super::types::{Report, Verdict, NS};
use super::util::{highpass, mean, rms, stddev, tls_line};

#[derive(Copy, Clone, Debug)]
pub struct SnapConfig {
    pub cpi: f64,
    /// Target counts of travel per bin. 8 counts makes the angle of a bin resolvable to
    /// ~ atan(0.5/8) = 3.6 degrees, fine enough for the angle histogram.
    pub min_bin_counts: f64,
    /// Reject the stroke as Inconclusive below this median speed (inches/second).
    pub min_median_ips: f64,
    /// Reject the stroke as Inconclusive below this total travel (inches).
    pub min_travel_in: f64,
    /// sigma_perp/L bands for the (calibration-dependent) straightness test.
    pub straightness_fail: f64,
    pub straightness_warn: f64,
    /// hf_perp_ratio bands (secondary, post-quantisation snappers only).
    pub hf_ratio_fail: f64,
    pub hf_ratio_warn: f64,
    /// hf_aniso bands (PRIMARY).
    pub aniso_fail: f64,
    pub aniso_warn: f64,
    /// Along-axis high-frequency RMS, in counts, below which hf_aniso has nothing
    /// to work with. The statistic is a RATIO of the across-stroke noise to the
    /// along-stroke noise, so if the sensor is quiet enough that the along-stroke
    /// figure is itself near the quantisation floor, the ratio is dominated by
    /// quantisation and a snapping device can sit at 1.0 and pass. Below this the
    /// test declares itself unavailable instead.
    pub aniso_min_along_rms: f64,
    /// Excess dy==0 fraction above the quantiser prediction.
    pub axis_lock_fail: f64,
    pub axis_lock_warn: f64,
}

impl Default for SnapConfig {
    fn default() -> Self {
        SnapConfig {
            cpi: 1600.0,
            min_bin_counts: 8.0,
            // 5 IPS: below this a stroke is a "creep" and the low-frequency straightness
            // test cannot separate a careful human from a snapper.
            min_median_ips: 5.0,
            min_travel_in: 2.0,
            // Freehand straight-line drawing: low-frequency hand wander is ~1-2% of stroke
            // length and physiological tremor adds ~0.1-0.5 mm RMS, so a genuine stroke
            // sits at sigma_perp/L ~ 0.005-0.03. 0.001 is 5x below the bottom of that
            // range and 0.003 is at its edge. THESE TWO NUMBERS ARE THE ONLY
            // HUMAN-CALIBRATED CONSTANTS IN THIS DETECTOR and the app should re-fit them
            // from the user's own known-good mouse; the HF test below does not need them.
            straightness_fail: 0.001,
            straightness_warn: 0.003,
            // Quantisation alone guarantees ratio ~ 1.0. Firmware that draws the line
            // gives ~0. 0.35 is a wide margin below anything a digitiser can produce.
            hf_ratio_fail: 0.35,
            hf_ratio_warn: 0.60,
            // Isotropy is a physical property of the sensor, not a tuned constant, so
            // the null is 1.0 with only sampling scatter around it. 0.55 and 0.75 sit
            // far outside that scatter (measured null spread below).
            aniso_fail: 0.55,
            aniso_warn: 0.75,
            // 1.0 count/report of along-stroke high-frequency residual. Below this
            // the ratio is dominated by the quantiser rather than by the sensor,
            // and the measured separation between clean and snapped collapses:
            // at 2.0 counts it is 1.000 vs 0.540, at 4.0 counts 0.971 vs 0.519,
            // but a quiet sensor puts both near 1 and the test loses its power.
            aniso_min_along_rms: 1.0,
            axis_lock_fail: 0.25,
            axis_lock_warn: 0.10,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SnapResult {
    pub verdict: Verdict,
    pub n_reports: usize,
    pub n_bins: usize,
    pub bin_ns: u64,
    pub travel_in: f64,
    pub median_ips: f64,
    /// RMS perpendicular residual from the TLS line, in counts.
    pub sigma_perp_counts: f64,
    /// sigma_perp / stroke length. Dimensionless straightness.
    pub straightness: f64,
    /// sd of bin-to-bin perpendicular increments, counts. Quantisation floor is 0.408.
    pub perp_step_sd: f64,
    /// RMS of the >15 Hz part of the perpendicular increments / 0.408.
    /// Secondary: only fires for firmware that snaps AFTER quantisation.
    pub hf_perp_ratio: f64,
    /// PRIMARY, calibration-free: HF perpendicular energy / HF along-axis energy.
    /// ~1.0 for any honest digitiser; << 1 when the perpendicular axis has been projected
    /// away by a snapper.
    pub hf_aniso: f64,
    /// Along-stroke high-frequency RMS, counts. This is the device measuring its
    /// own sensor noise, and it is what `hf_aniso` is a ratio against.
    pub hf_along_rms: f64,
    /// False when the sensor is too quiet for `hf_aniso` to mean anything. The
    /// primary test is then reported as unavailable rather than as a pass.
    pub aniso_applicable: bool,
    /// Observed fraction of reports with dy==0 while dx!=0.
    pub axis_lock_frac: f64,
    /// Fraction predicted by integer quantisation of the observed mean |dy|.
    pub axis_lock_expected: f64,
    pub axis_lock_excess: f64,
    /// Circular concentration of per-bin angles, |mean(e^{i theta})| in [0,1].
    pub angle_r_bar: f64,
    /// Circular standard deviation of per-bin angles, degrees.
    pub angle_sd_deg: f64,
    /// Fraction of bin angles within 1 degree of a multiple of 45 degrees.
    pub angle_on_octant_frac: f64,
    pub note: &'static str,
}

pub fn analyze_snap(reports: &[Report], cfg: &SnapConfig) -> SnapResult {
    let mut out = SnapResult {
        verdict: Verdict::Inconclusive, n_reports: 0, n_bins: 0, bin_ns: 0,
        travel_in: 0.0, median_ips: 0.0, sigma_perp_counts: 0.0, straightness: f64::NAN,
        perp_step_sd: f64::NAN, hf_perp_ratio: f64::NAN, hf_aniso: f64::NAN,
        hf_along_rms: f64::NAN, aniso_applicable: false,
        axis_lock_frac: 0.0, axis_lock_expected: 0.0, axis_lock_excess: 0.0,
        angle_r_bar: f64::NAN, angle_sd_deg: f64::NAN, angle_on_octant_frac: 0.0, note: "",
    };
    if !super::types::is_monotonic(reports) {
        out.note = super::types::NOT_MONOTONIC; return out;
    }
    let Some((a, b)) = dominant_span(reports, MotionGate::default()) else {
        out.note = "no stroke found"; return out;
    };
    let seg = &reports[a..b];
    out.n_reports = seg.len();
    if seg.len() < 64 { out.note = "stroke too short (<64 reports)"; return out; }

    let dur = seg[seg.len() - 1].t_ns.saturating_sub(seg[0].t_ns) as f64 / NS;
    let path: f64 = seg.iter().map(|r| r.mag()).sum();
    out.travel_in = path / cfg.cpi;
    if dur <= 0.0 { out.note = "zero-duration stroke"; return out; }

    // ---- (a) raw-report axis lock ----
    // Computed AFTER the line fit, further down, because the only correct predictor of
    // the expected |dy| rate is the FITTED STROKE SLOPE times the observed |dx| --
    // not the observed |dy| itself. Predicting from observed |dy| is circular: a device
    // that pins dy to zero also drives the prediction to zero, and the excess vanishes.

    // ---- choose a bin so each bin carries >= min_bin_counts of travel ----
    let counts_per_s = path / dur;
    if counts_per_s <= 0.0 { out.note = "no travel"; return out; }
    let bin_s = (cfg.min_bin_counts / counts_per_s).max(1.0e-4); // >= 0.1 ms
    out.bin_ns = (bin_s * NS) as u64;
    let bins = bin_deltas(seg, out.bin_ns);
    // Keep only bins that actually contain reports (a mid-stroke hole would otherwise
    // inject a fake zero-motion sample).
    let bins: Vec<_> = bins.into_iter().filter(|b| b.3 > 0).collect();
    out.n_bins = bins.len();
    if bins.len() < 24 { out.note = "too few bins; stroke too short or too slow"; return out; }

    // per-bin speed -> median IPS, and the slow-stroke guard
    let bin_speeds: Vec<f64> = bins.iter()
        .map(|b| (b.1 * b.1 + b.2 * b.2).sqrt() / bin_s / cfg.cpi).collect();
    out.median_ips = super::util::median(&bin_speeds);

    // ---- cumulative path and TLS fit ----
    let mut pts = Vec::with_capacity(bins.len());
    let (mut cx, mut cy) = (0.0f64, 0.0f64);
    for b in &bins { cx += b.1; cy += b.2; pts.push((cx, cy)); }
    let (c, u, sperp) = tls_line(&pts);
    out.sigma_perp_counts = sperp;
    let len_counts = (cx * cx + cy * cy).sqrt();
    if len_counts <= 0.0 { out.note = "net displacement zero"; return out; }
    out.straightness = sperp / len_counts;

    // perpendicular residual series and its increments
    let (px, py) = (-u.1, u.0);
    let perp: Vec<f64> = pts.iter().map(|p| (p.0 - c.0) * px + (p.1 - c.1) * py).collect();
    let dperp: Vec<f64> = perp.windows(2).map(|w| w[1] - w[0]).collect();
    out.perp_step_sd = stddev(&dperp);

    // ---- HF band of the perpendicular increments ----
    // Bin rate is 1/bin_s Hz. High-pass at ~15 Hz by subtracting a moving average of
    // length w where the MA's first null is at bin_rate/w; choose w so the corner is
    // ~15 Hz, clamped to [3, 41].
    let bin_rate = 1.0 / bin_s;
    let w = ((bin_rate / 15.0).round() as usize).clamp(3, 41);
    let (hf, weff) = highpass(&dperp, w);
    // A length-w moving-average subtraction removes a fraction 1/w of white variance;
    // divide it back out so the ratio is 1.0 for white input.
    let corr = (1.0 - 1.0 / weff as f64).max(0.25).sqrt();
    // Quantisation floor: perpendicular position error is U(-0.5,0.5) so its first
    // difference has sd sqrt(2/12) = 0.4082 counts, independent of everything else.
    const QUANT_FLOOR: f64 = 0.40825;
    out.hf_perp_ratio = rms(&hf) / (corr * QUANT_FLOOR);

    // PRIMARY: the same measurement made ALONG the stroke, used as the device's own
    // self-reported noise level. The high-pass also removes the speed profile, so what
    // is left along the axis is exactly the device's own high-frequency error.
    //
    // BOTH SERIES MUST BE COMPUTED PER REPORT, NOT PER BIN. Binning puts a
    // "how many reports landed in this bin" aliasing term into the ALONG axis (it is
    // proportional to the speed) that the PERP axis does not get, which by itself drove
    // hf_aniso to 0.5 on a perfectly clean 8 kHz stream and produced 165/250 false
    // FAILures before this was fixed.
    let rr = {
        let iv: Vec<f64> = seg.windows(2)
            .map(|q| q[1].t_ns.saturating_sub(q[0].t_ns) as f64).collect();
        // Modal slot, not the median: on a device that drops reports the median
        // interval lands on a multiple of the true one, and this sets the
        // high-pass window.
        let ns = crate::core::polling::nominal_interval_ns(&iv);
        if ns > 0.0 { NS / ns } else { 1000.0 }
    };
    let wr = ((rr / 20.0).round() as usize).clamp(3, 501);
    let dperp_r: Vec<f64> = seg.iter()
        .map(|r| (r.dx as f64) * px + (r.dy as f64) * py).collect();
    let dalong_r: Vec<f64> = seg.iter()
        .map(|r| (r.dx as f64) * u.0 + (r.dy as f64) * u.1).collect();
    let (hfp, _) = highpass(&dperp_r, wr);
    let (hfa, _) = highpass(&dalong_r, wr);
    let ra = rms(&hfa);
    out.hf_along_rms = ra;
    // Only meaningful when the device has enough of its own noise to measure.
    out.aniso_applicable = ra >= cfg.aniso_min_along_rms;
    out.hf_aniso = if ra > 1e-9 && out.aniso_applicable {
        rms(&hfp) / ra
    } else {
        f64::NAN
    };

    // ---- (a) axis lock, predicted from the fitted slope ----
    // Driver = the dominant axis, driven = the other one. For each report the expected
    // |driven| increment is |driver_i| * |slope|, and a residual-accumulator quantiser
    // emits zero with probability max(0, 1 - m_i). Averaging that over reports gives the
    // fraction of zeros a HONEST device must produce; anything above it is axis lock.
    {
        let x_dom = u.0.abs() >= u.1.abs();
        let slope = if x_dom { (u.1 / u.0).abs() } else { (u.0 / u.1).abs() };
        let drv: Vec<f64> = seg.iter()
            .map(|r| if x_dom { r.dx as f64 } else { r.dy as f64 }).collect();
        let drn: Vec<i32> = seg.iter()
            .map(|r| if x_dom { r.dy } else { r.dx }).collect();
        let idx: Vec<usize> = (0..seg.len()).filter(|i| drv[*i] != 0.0).collect();
        if idx.len() >= 32 {
            let n0 = idx.iter().filter(|i| drn[**i] == 0).count();
            out.axis_lock_frac = n0 as f64 / idx.len() as f64;
            out.axis_lock_expected = idx.iter()
                .map(|i| (1.0 - (drv[*i].abs() * slope)).clamp(0.0, 1.0))
                .sum::<f64>() / idx.len() as f64;
            out.axis_lock_excess = out.axis_lock_frac - out.axis_lock_expected;
        }
    }

    // ---- angle statistics on bins ----
    let angs: Vec<f64> = bins.iter()
        .filter(|b| (b.1 * b.1 + b.2 * b.2).sqrt() >= cfg.min_bin_counts * 0.5)
        .map(|b| b.2.atan2(b.1)).collect();
    if angs.len() >= 16 {
        let cbar = mean(&angs.iter().map(|a| a.cos()).collect::<Vec<_>>());
        let sbar = mean(&angs.iter().map(|a| a.sin()).collect::<Vec<_>>());
        let r = (cbar * cbar + sbar * sbar).sqrt().min(1.0);
        out.angle_r_bar = r;
        out.angle_sd_deg = if r > 0.0 { (-2.0 * r.ln()).sqrt().to_degrees() } else { f64::INFINITY };
        let oct = std::f64::consts::FRAC_PI_4;
        let tol = 1.0f64.to_radians();
        out.angle_on_octant_frac = angs.iter().filter(|a| {
            let m = (**a / oct).round() * oct;
            (**a - m).abs() < tol
        }).count() as f64 / angs.len() as f64;
    }

    // ---- guards, then verdict ----
    if out.travel_in < cfg.min_travel_in {
        out.note = "stroke shorter than min_travel_in; draw a longer line";
        return out;
    }
    if out.median_ips < cfg.min_median_ips {
        out.note = "stroke too slow to distinguish careful drawing from snapping";
        return out;
    }

    let mut v = Verdict::Pass;
    let mut note = "no snapping signature";
    // PRIMARY test: high-frequency isotropy.
    if !out.aniso_applicable {
        // Never silently pass on a test that did not run.
        v = Verdict::Inconclusive;
        note = "this sensor is too quiet for the primary test: with almost no \
                high-frequency noise along the stroke there is nothing for the \
                across-stroke comparison to measure. The secondary statistics below \
                still apply.";
    }
    if out.hf_aniso.is_finite() {
        if out.hf_aniso < cfg.aniso_fail {
            v = Verdict::Fail;
            note = "high-frequency error is anisotropic: the across-stroke component of the sensor's own noise has been projected away (angle snapping)";
        } else if out.hf_aniso < cfg.aniso_warn {
            v = v.worst(Verdict::Warn);
            note = "high-frequency error is mildly anisotropic";
        }
    }
    // Secondary: post-quantisation snappers.
    if out.hf_perp_ratio < cfg.hf_ratio_fail {
        v = Verdict::Fail;
        note = "perpendicular high-frequency energy below the quantisation floor: the path is synthesised, not digitised";
    } else if out.hf_perp_ratio < cfg.hf_ratio_warn {
        v = v.worst(Verdict::Warn);
        if note == "no snapping signature" { note = "perpendicular high-frequency energy suppressed"; }
    }
    // Axis lock in excess of what quantisation explains.
    if out.axis_lock_excess > cfg.axis_lock_fail {
        v = Verdict::Fail;
        note = "dy is pinned to zero far more often than quantisation explains (axis lock)";
    } else if out.axis_lock_excess > cfg.axis_lock_warn {
        v = v.worst(Verdict::Warn);
    }
    // Human-calibrated straightness, last, and only ever escalates to Warn on its own.
    if out.straightness < cfg.straightness_fail {
        v = v.worst(Verdict::Fail);
        if note == "no snapping signature" { note = "line is straighter than a hand can draw"; }
    } else if out.straightness < cfg.straightness_warn {
        v = v.worst(Verdict::Warn);
    }
    if out.angle_on_octant_frac > 0.60 {
        v = v.worst(Verdict::Fail);
        note = "instantaneous angle is quantised onto 45-degree octants";
    }
    out.verdict = v;
    out.note = note;
    out
}
