//! The sensor detectors, checked against a simulator with known ground truth.
//!
//! Every threshold in this module is a claim about a physical device, and the
//! only honest way to check one without a device is to generate a stream whose
//! truth is known and see whether the detector recovers it. Two things are
//! asserted throughout: that a clean mouse does not fail, and that a defective
//! one does. The first matters more. A false alarm on working hardware is the
//! failure mode that makes a measurement tool useless.

use super::mousesim::*;
use super::types::{Report, Verdict};
use super::*;
use crate::core::sim::Rng;

fn rng(seed: u64) -> Rng {
    Rng::new(seed)
}

// ---------------------------------------------------------------- CPI

#[test]
fn cpi_recovers_a_known_count_per_inch() {
    let sim = MouseSim { cpi: 1600.0, ..Default::default() };
    let mut r = rng(1);
    let mut errs = Vec::new();
    for i in 0..40 {
        let traj = SwipeTraj::new(&mut r, 4.0, 0.45, 0.15 * i as f64);
        let reports = sim.render(&traj, &mut r);
        let cfg = cpi::CpiConfig::new(1600.0, 4.0);
        let out = cpi::analyze_cpi(&reports, &cfg);
        assert_ne!(out.verdict, Verdict::Fail, "clean mouse failed CPI: {}", out.note);
        if out.measured_cpi.is_finite() {
            errs.push((out.measured_cpi - 1600.0) / 1600.0);
        }
    }
    assert!(errs.len() >= 35, "too few usable swipes: {}", errs.len());
    let bias = errs.iter().sum::<f64>() / errs.len() as f64;
    assert!(bias.abs() < 0.005, "CPI bias {:.4} exceeds 0.5%", bias);
}

#[test]
fn cpi_flags_a_sensor_that_is_ten_percent_off() {
    // The device really counts at 1760 while claiming 1600.
    let sim = MouseSim { cpi: 1760.0, ..Default::default() };
    let mut r = rng(2);
    let mut fails = 0;
    for i in 0..30 {
        let traj = SwipeTraj::new(&mut r, 4.0, 0.45, 0.2 * i as f64);
        let reports = sim.render(&traj, &mut r);
        let out = cpi::analyze_cpi(&reports, &cpi::CpiConfig::new(1600.0, 4.0));
        if out.verdict == Verdict::Fail {
            fails += 1;
        }
    }
    assert!(fails >= 28, "only {fails}/30 detected a 10% CPI error");
}

#[test]
fn cpi_uses_net_displacement_not_arc_length() {
    // Arc length is biased high by sensor noise and the bias never averages
    // out, because E|d| > |E d| for every noisy report. This asserts the
    // estimator actually shipped is the unbiased one.
    let sim = MouseSim { cpi: 1600.0, noise_counts_1khz: 1.5, ..Default::default() };
    let mut r = rng(3);
    let traj = SwipeTraj::new(&mut r, 4.0, 0.45, 0.0);
    let reports = sim.render(&traj, &mut r);
    let out = cpi::analyze_cpi(&reports, &cpi::CpiConfig::new(1600.0, 4.0));
    assert!(out.l_path > out.l_net, "arc length should exceed the chord");
    assert!(
        (out.measured_cpi - out.l_net / 4.0).abs() < 1e-6,
        "measured CPI is not derived from net displacement"
    );
}

#[test]
fn cpi_refuses_a_swipe_that_is_too_short_to_answer() {
    let sim = MouseSim::default();
    let mut r = rng(4);
    let traj = SwipeTraj::new(&mut r, 0.15, 0.2, 0.0);
    let reports = sim.render(&traj, &mut r);
    let out = cpi::analyze_cpi(&reports, &cpi::CpiConfig::new(1600.0, 0.15));
    assert_eq!(out.verdict, Verdict::Inconclusive);
    assert!(!out.note.is_empty(), "a refusal must say why");
}

// ---------------------------------------------------------------- drift

#[test]
fn drift_passes_a_mouse_that_is_merely_noisy() {
    // Zero-mean jitter is not drift, however much of it there is. A random
    // walk of N steps has |sum| growing as sqrt(N), which is not small in
    // absolute counts, so this is exactly the case a naive test gets wrong.
    let sim = MouseSim { idle_jitter_cps: 4.0, ..Default::default() };
    let mut r = rng(5);
    let mut called = 0;
    for _ in 0..60 {
        let reports = sim.render(&StillTraj { dur_s: 15.0 }, &mut r);
        let out = drift::analyze_drift(&reports, 15.0, &drift::DriftConfig::default());
        if out.drift_detected {
            called += 1;
        }
    }
    assert!(called <= 2, "{called}/60 false drift calls on pure jitter");
}

#[test]
fn drift_detects_a_real_bias_and_measures_its_rate() {
    let sim = MouseSim { drift_cps: (3.0, 0.0), idle_jitter_cps: 4.0, ..Default::default() };
    let mut r = rng(6);
    let mut rates = Vec::new();
    for _ in 0..30 {
        let reports = sim.render(&StillTraj { dur_s: 15.0 }, &mut r);
        let out = drift::analyze_drift(&reports, 15.0, &drift::DriftConfig::default());
        assert!(out.drift_detected, "missed a 3 count/s drift");
        rates.push(out.drift_cps);
    }
    let m = super::util::median(&rates);
    assert!((m - 3.0).abs() < 0.5, "recovered drift {m:.2} c/s, truth 3.00");
}

#[test]
fn drift_passes_a_silent_mouse_without_pretending_to_measure() {
    let sim = MouseSim::default();
    let mut r = rng(7);
    let reports = sim.render(&StillTraj { dur_s: 15.0 }, &mut r);
    let out = drift::analyze_drift(&reports, 15.0, &drift::DriftConfig::default());
    assert_eq!(out.verdict, Verdict::Pass);
    assert!(!out.drift_detected);
}

#[test]
fn drift_refuses_a_capture_shorter_than_its_protocol() {
    let sim = MouseSim::default();
    let mut r = rng(8);
    let reports = sim.render(&StillTraj { dur_s: 1.0 }, &mut r);
    let out = drift::analyze_drift(&reports, 1.0, &drift::DriftConfig::default());
    assert_eq!(out.verdict, Verdict::Inconclusive);
}

// ---------------------------------------------------------------- snapping

#[test]
fn snapping_does_not_fire_on_a_hand_drawn_line() {
    // The one that matters: a real person drawing a real line must not be
    // told their mouse corrects their path.
    let sim = MouseSim { noise_counts_1khz: 2.0, ..Default::default() };
    let mut r = rng(9);
    let mut fails = 0;
    for i in 0..40 {
        let traj = SwipeTraj::new(&mut r, 5.0, 0.5, 0.1 * i as f64);
        let reports = sim.render(&traj, &mut r);
        let out = snap::analyze_snap(&reports, &snap::SnapConfig::default());
        if out.verdict == Verdict::Fail {
            fails += 1;
        }
    }
    assert_eq!(fails, 0, "{fails}/40 clean strokes called snapping");
}

#[test]
fn snapping_is_detected_when_the_firmware_straightens_the_path() {
    let sim = MouseSim { noise_counts_1khz: 2.0, snap_perp_keep: 0.0, ..Default::default() };
    let mut r = rng(10);
    let mut fails = 0;
    for i in 0..40 {
        let traj = SwipeTraj::new(&mut r, 5.0, 0.5, 0.1 * i as f64);
        let reports = sim.render(&traj, &mut r);
        let out = snap::analyze_snap(&reports, &snap::SnapConfig::default());
        if out.verdict == Verdict::Fail {
            fails += 1;
        }
    }
    assert!(fails >= 30, "only {fails}/40 fully snapped strokes detected");
}

#[test]
fn snapping_reports_itself_unavailable_on_a_sensor_too_quiet_to_judge() {
    // The primary statistic is a ratio against the sensor's own noise. With
    // almost no noise there is nothing to take a ratio of, and the honest
    // answer is that the test did not run, not that the mouse passed.
    let sim = MouseSim { noise_counts_1khz: 0.05, snap_perp_keep: 0.0, ..Default::default() };
    let mut r = rng(11);
    let traj = SwipeTraj::new(&mut r, 5.0, 0.5, 0.3);
    let reports = sim.render(&traj, &mut r);
    let out = snap::analyze_snap(&reports, &snap::SnapConfig::default());
    assert!(!out.aniso_applicable, "along-axis RMS {:.3}", out.hf_along_rms);
    assert!(!out.hf_aniso.is_finite(), "an unavailable test must not report a number");
    assert_ne!(out.verdict, Verdict::Pass, "must not pass on a test that did not run");
}

#[test]
fn snapping_refuses_a_stroke_too_slow_to_separate_care_from_correction() {
    let sim = MouseSim::default();
    let mut r = rng(12);
    let traj = SwipeTraj::new(&mut r, 2.5, 3.0, 0.0);
    let reports = sim.render(&traj, &mut r);
    let out = snap::analyze_snap(&reports, &snap::SnapConfig::default());
    assert_eq!(out.verdict, Verdict::Inconclusive, "note: {}", out.note);
}

// ---------------------------------------------------------------- smoothing

fn hard_stop(ips: f64) -> HardStopTraj {
    HardStopTraj { ips, glide_s: 0.6, after_s: 0.5, dir_rad: 0.3, tremor_in: 0.010, ph: 0.7 }
}

#[test]
fn smoothing_passes_an_unfiltered_mouse_whose_sensor_keeps_talking() {
    // This is the regression test for the defect that made the whole port
    // worth auditing. With sensor noise that does not stop when the hand
    // does, the tail statistic had no termination rule and an UNFILTERED
    // mouse measured a 47.7 ms interpolation tail at 2 counts/second of idle
    // jitter, rising to 224 ms at 40. Both are inside the drift detector's
    // own healthy band, so this fired on hardware with nothing wrong with it.
    for jitter in [0.0, 2.0, 10.0, 40.0] {
        let sim = MouseSim { ema_alpha: 1.0, idle_jitter_cps: jitter, ..Default::default() };
        let mut r = rng(13);
        let mut fails = 0;
        let mut worst_tail = 0.0f64;
        for _ in 0..20 {
            let reports = sim.render(&hard_stop(18.0), &mut r);
            let out = smooth::analyze_smoothing(&reports, &smooth::SmoothConfig::default());
            worst_tail = worst_tail.max(out.tail_ms);
            if out.verdict == Verdict::Fail {
                fails += 1;
            }
        }
        assert_eq!(
            fails, 0,
            "{fails}/20 unfiltered runs failed at {jitter} c/s idle jitter, \
             worst tail {worst_tail:.1} ms"
        );
    }
}

#[test]
fn smoothing_is_detected_and_its_time_constant_recovered() {
    // tau = -dt / ln(1 - alpha) at dt = 1 ms.
    for (alpha, truth_ms) in [(0.50, 1.44), (0.30, 2.80), (0.20, 4.48), (0.10, 9.49)] {
        let sim = MouseSim { ema_alpha: alpha, ..Default::default() };
        let mut r = rng(14);
        let mut fails = 0;
        let mut taus = Vec::new();
        for _ in 0..20 {
            let reports = sim.render(&hard_stop(18.0), &mut r);
            let out = smooth::analyze_smoothing(&reports, &smooth::SmoothConfig::default());
            if out.verdict == Verdict::Fail {
                fails += 1;
            }
            if out.decay_tau_ms.is_finite() {
                taus.push(out.decay_tau_ms);
            }
        }
        assert!(fails >= 18, "alpha {alpha}: only {fails}/20 detected");
        if !taus.is_empty() {
            let m = super::util::median(&taus);
            assert!(
                (m - truth_ms).abs() / truth_ms < 0.25,
                "alpha {alpha}: recovered tau {m:.2} ms against truth {truth_ms:.2} ms"
            );
        }
    }
}

#[test]
fn smoothing_refuses_a_glide_too_fast_to_measure() {
    let sim = MouseSim { ema_alpha: 1.0, ..Default::default() };
    let mut r = rng(15);
    let reports = sim.render(&hard_stop(70.0), &mut r);
    let out = smooth::analyze_smoothing(&reports, &smooth::SmoothConfig::default());
    assert_eq!(out.verdict, Verdict::Inconclusive, "note: {}", out.note);
}

#[test]
fn smoothing_rate_estimate_survives_a_device_that_drops_reports() {
    // The report rate scales both the recovered time constant and the
    // high-pass corner. The median interval, which this used to use, lands on
    // twice the truth once most gaps are two slots wide.
    let sim = MouseSim { ema_alpha: 1.0, drop_prob: 0.6, ..Default::default() };
    let mut r = rng(16);
    let reports = sim.render(&hard_stop(18.0), &mut r);
    let out = smooth::analyze_smoothing(&reports, &smooth::SmoothConfig::default());
    assert!(
        (out.report_rate_hz - 1000.0).abs() < 100.0,
        "recovered {:.0} Hz at a 60% drop rate, truth 1000 Hz",
        out.report_rate_hz
    );
}

// ---------------------------------------------------------------- tracking

#[test]
fn tracking_reports_a_lower_bound_when_the_sensor_never_failed() {
    // A mouse that tracked everything the user could do has not been shown to
    // have a limit. Reporting the fastest achieved speed as "the maximum" is
    // the wrong answer, and this asserts it is flagged as a bound instead.
    let sim = MouseSim { max_track_ips: None, ..Default::default() };
    let mut r = rng(17);
    let reports = sim.render(&ramp(&mut r), &mut r);
    let out = tracking::analyze_tracking(&reports, &tracking::TrackConfig::default());
    assert!(out.bounded_below, "no limit exists, so the result must be a lower bound");
    assert_ne!(out.verdict, Verdict::Fail);
}

fn ramp(r: &mut Rng) -> RampTraj {
    // Swipes of 6 inches taking successively less time: 20 to 300 IPS.
    let mut swipes = Vec::new();
    for i in 0..12 {
        let ips = 20.0 + 26.0 * i as f64;
        swipes.push((6.0, 6.0 / ips));
    }
    RampTraj { swipes, gap_s: 0.25, ph: r.unit() * 6.28 }
}

#[test]
fn tracking_finds_the_speed_at_which_the_sensor_gives_up() {
    for truth in [60.0, 120.0] {
        let sim = MouseSim { max_track_ips: Some(truth), ..Default::default() };
        let mut r = rng(18);
        let mut ests = Vec::new();
        for _ in 0..12 {
            let reports = sim.render(&ramp(&mut r), &mut r);
            let out = tracking::analyze_tracking(&reports, &tracking::TrackConfig::default());
            if out.max_tracking_ips > 0.0 && !out.bounded_below {
                ests.push(out.max_tracking_ips);
            }
        }
        assert!(ests.len() >= 8, "truth {truth}: only {} runs found a limit", ests.len());
        let m = super::util::median(&ests);
        assert!(
            (m - truth).abs() / truth < 0.30,
            "truth {truth} IPS, recovered {m:.1} IPS"
        );
    }
}

#[test]
fn tracking_identifies_a_report_field_that_is_too_narrow() {
    // An 8-bit field saturates at +-127 counts per report, which at 1600 CPI
    // and 1 kHz is a hard ceiling of 79.4 IPS that has nothing to do with the
    // sensor. Calling that a sensor limit would blame the wrong component.
    let sim = MouseSim { field_bound: Some(127), ..Default::default() };
    let mut r = rng(19);
    let reports = sim.render(&ramp(&mut r), &mut r);
    let out = tracking::analyze_tracking(&reports, &tracking::TrackConfig::default());
    assert_eq!(out.field.matched_bound, Some(127), "field width not identified");
    assert!(out.field.saturating, "saturation not reported");
}

#[test]
fn tracking_does_not_invent_a_field_limit_that_was_never_reached() {
    let sim = MouseSim { field_bound: Some(127), ..Default::default() };
    let mut r = rng(20);
    let traj = SwipeTraj::new(&mut r, 3.0, 1.2, 0.0);
    let reports = sim.render(&traj, &mut r);
    let out = tracking::analyze_tracking(&reports, &tracking::TrackConfig::default());
    assert!(!out.field.saturating, "a bound never reached is not a saturation");
}

// ---------------------------------------------------------------- shared

#[test]
fn every_detector_refuses_a_capture_whose_clock_went_backwards() {
    // One out-of-order report used to panic a debug build outright and, in
    // release, wrap a u64 subtraction to 1.8e19 ns and turn a clean stream
    // into a Warn. Merging two devices' streams produces exactly this.
    let sim = MouseSim::default();
    let mut r = rng(21);
    let traj = SwipeTraj::new(&mut r, 4.0, 0.45, 0.0);
    let mut reports = sim.render(&traj, &mut r);
    assert!(reports.len() > 300);
    let mid = reports.len() / 2;
    reports[mid].t_ns = reports[mid].t_ns.saturating_sub(50_000_000);

    assert_eq!(
        cpi::analyze_cpi(&reports, &cpi::CpiConfig::new(1600.0, 4.0)).verdict,
        Verdict::Inconclusive
    );
    assert_eq!(
        drift::analyze_drift(&reports, 15.0, &drift::DriftConfig::default()).verdict,
        Verdict::Inconclusive
    );
    assert_eq!(
        snap::analyze_snap(&reports, &snap::SnapConfig::default()).verdict,
        Verdict::Inconclusive
    );
    assert_eq!(
        smooth::analyze_smoothing(&reports, &smooth::SmoothConfig::default()).verdict,
        Verdict::Inconclusive
    );
    assert_eq!(
        tracking::analyze_tracking(&reports, &tracking::TrackConfig::default()).verdict,
        Verdict::Inconclusive
    );
}

#[test]
fn no_detector_panics_on_a_degenerate_capture() {
    let empty: Vec<Report> = vec![];
    let one = vec![Report::motion(0, 0, 0)];
    let same = vec![Report::motion(5, 1, 1); 400];
    for reports in [empty, one, same] {
        let _ = cpi::analyze_cpi(&reports, &cpi::CpiConfig::new(1600.0, 4.0));
        let _ = drift::analyze_drift(&reports, 15.0, &drift::DriftConfig::default());
        let _ = snap::analyze_snap(&reports, &snap::SnapConfig::default());
        let _ = smooth::analyze_smoothing(&reports, &smooth::SmoothConfig::default());
        let _ = tracking::analyze_tracking(&reports, &tracking::TrackConfig::default());
    }
}

