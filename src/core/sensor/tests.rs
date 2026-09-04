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
fn cpi_refuses_a_swipe_the_mouse_dropped_out_of() {
    // The failure that sent a working mouse's CPI to 88% of nominal, run after
    // run, on hardware that disconnects at random. Deleting a slice out of the
    // middle of a swipe takes the same fraction off the path and the chord, so
    // wobble stays at 1.00 and every other guard is satisfied. Only the hole in
    // the timestamps gives it away, and until this guard existed nothing looked.
    let sim = MouseSim { cpi: 1600.0, ..Default::default() };
    let mut r = rng(7);
    let traj = SwipeTraj::new(&mut r, 4.0, 0.45, 0.0);
    let clean = sim.render(&traj, &mut r);
    let cfg = cpi::CpiConfig::new(1600.0, 4.0);
    assert_ne!(
        cpi::analyze_cpi(&clean, &cfg).verdict,
        Verdict::Inconclusive,
        "the unbroken swipe must still be usable, or this guard is just noise"
    );

    // Drop a fifth of the swipe out of the middle, as a disconnect would.
    let a = clean.len() * 2 / 5;
    let b = clean.len() * 3 / 5;
    let holed: Vec<_> = clean[..a].iter().chain(&clean[b..]).cloned().collect();

    let out = cpi::analyze_cpi(&holed, &cfg);
    assert!(
        out.wobble < 1.15,
        "premise of the test: the hole must be invisible to the wobble guard, was {:.3}",
        out.wobble
    );
    assert!(
        out.measured_cpi < 1600.0 * 0.95,
        "premise of the test: the hole must deflate the reading, was {:.0}",
        out.measured_cpi
    );
    assert_eq!(
        out.verdict,
        Verdict::Inconclusive,
        "a swipe with a hole in it must be refused, not reported: {}",
        out.note
    );
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


// ---------------------------------------------------------------- scroll

fn scroll_stream(sim: &ScrollSim, detents: usize, r: &mut Rng) -> Vec<Report> {
    // Half up then half down, which is what a person actually does and what the
    // direction counts have to separate.
    let mut a = sim.render(detents / 2, 1, r);
    let last = a.last().map(|x| x.t_ns).unwrap_or(0);
    let mut b = sim.render(detents - detents / 2, -1, r);
    for x in b.iter_mut() {
        x.t_ns += last + 400_000_000;
    }
    a.append(&mut b);
    a
}

#[test]
fn scroll_infers_the_counts_per_detent_of_every_common_wheel() {
    // What one detent is worth is device and platform dependent, so assuming it
    // would be wrong on most hardware. These four shapes are all real.
    let cases: [(&str, ScrollSim, f64); 4] = [
        ("classic, 1 count per detent", ScrollSim::default(), 1.0),
        (
            "Windows WHEEL_DELTA, 120 per detent",
            ScrollSim { counts_per_detent: 120, ..Default::default() },
            120.0,
        ),
        (
            "high resolution, 120 split across 4 reports",
            ScrollSim { counts_per_detent: 120, reports_per_detent: 4, ..Default::default() },
            120.0,
        ),
        (
            "HID++ high resolution, 8 sub-counts per detent",
            ScrollSim { counts_per_detent: 8, reports_per_detent: 8, ..Default::default() },
            8.0,
        ),
    ];
    for (name, sim, truth) in cases {
        let mut r = rng(30);
        let reports = scroll_stream(&sim, 60, &mut r);
        let out = scroll::analyze_scroll(&reports, &scroll::ScrollConfig::default());
        assert_eq!(out.quantum, truth, "{name}: inferred {}", out.quantum);
        assert!(!out.continuous, "{name} was called continuous");
    }
}

#[test]
fn scroll_counts_detents_in_both_directions() {
    for sim in [
        ScrollSim::default(),
        ScrollSim { counts_per_detent: 120, reports_per_detent: 4, ..Default::default() },
    ] {
        let mut r = rng(31);
        let reports = scroll_stream(&sim, 80, &mut r);
        let out = scroll::analyze_scroll(&reports, &scroll::ScrollConfig::default());
        assert_eq!(out.detents_up, 40, "up count wrong");
        assert_eq!(out.detents_down, 40, "down count wrong");
        assert_eq!(out.verdict, Verdict::Pass, "note: {}", out.note);
    }
}

#[test]
fn scroll_detects_an_encoder_that_reverses() {
    let sim = ScrollSim { reverse_prob: 0.03, ..Default::default() };
    let mut r = rng(32);
    let mut flagged = 0;
    for _ in 0..40 {
        let reports = scroll_stream(&sim, 150, &mut r);
        let out = scroll::analyze_scroll(&reports, &scroll::ScrollConfig::default());
        if out.verdict != Verdict::Pass {
            flagged += 1;
        }
    }
    assert!(flagged >= 36, "only {flagged}/40 runs flagged a 3% reversal rate");
}

#[test]
fn scroll_detects_skipped_steps() {
    let sim = ScrollSim { skip_prob: 0.08, ..Default::default() };
    let mut r = rng(33);
    let mut flagged = 0;
    for _ in 0..40 {
        let reports = scroll_stream(&sim, 150, &mut r);
        let out = scroll::analyze_scroll(&reports, &scroll::ScrollConfig::default());
        if out.verdict != Verdict::Pass {
            flagged += 1;
        }
    }
    assert!(flagged >= 36, "only {flagged}/40 runs flagged an 8% skip rate");
}

#[test]
fn scroll_does_not_call_a_fast_flick_a_skip() {
    // A genuine flick merges detents, which looks exactly like a double step.
    // What separates them is that a flick also shortens the gap, so a real
    // skip is a double that arrives at an ordinary cadence.
    let sim = ScrollSim { cadence_ms: 25.0, cadence_jitter_ms: 6.0, ..Default::default() };
    let mut r = rng(34);
    let mut failed = 0;
    for _ in 0..40 {
        let reports = scroll_stream(&sim, 120, &mut r);
        let out = scroll::analyze_scroll(&reports, &scroll::ScrollConfig::default());
        if out.verdict == Verdict::Fail {
            failed += 1;
        }
    }
    assert_eq!(failed, 0, "{failed}/40 clean fast scrolls called defective");
}

#[test]
fn scroll_refuses_to_count_detents_on_a_free_spinning_wheel() {
    // A wheel with no detents has no detent count. Reporting one would be an
    // invented number, so the analysis says so instead.
    let sim = ScrollSim { continuous: true, ..Default::default() };
    let mut r = rng(35);
    let reports = scroll_stream(&sim, 120, &mut r);
    let out = scroll::analyze_scroll(&reports, &scroll::ScrollConfig::default());
    assert!(out.continuous, "quantum {} coverage {}", out.quantum, out.quantum_coverage);
    assert_eq!(out.verdict, Verdict::Inconclusive);
}

#[test]
fn scroll_analyses_the_tilt_wheel_separately() {
    let sim = ScrollSim { horizontal: true, ..Default::default() };
    let mut r = rng(36);
    let reports = scroll_stream(&sim, 60, &mut r);
    let cfg = scroll::ScrollConfig::default();
    let h = scroll::analyze_axis(&reports, scroll::Axis::Horizontal, &cfg);
    assert_eq!(h.detents_up + h.detents_down, 60);
    // The vertical wheel saw nothing, and must not borrow the horizontal one's
    // data to claim it did.
    let v = scroll::analyze_axis(&reports, scroll::Axis::Vertical, &cfg);
    assert_eq!(v.verdict, Verdict::Inconclusive);
    assert_eq!(v.detents_up + v.detents_down, 0);
}

#[test]
fn scroll_cluster_gap_search_is_linear_enough_for_the_interface_thread() {
    // The split search used to recompute each candidate's scatter from scratch,
    // which is quadratic: 665 ms at 40k wheel reports and 2.72 s at 80k, on the
    // thread that draws the window. A free-spinning high-resolution wheel
    // reaches 80k in under two minutes.
    let sim = ScrollSim { counts_per_detent: 8, reports_per_detent: 8, cadence_ms: 30.0,
                          cadence_jitter_ms: 8.0, ..Default::default() };
    let mut r = rng(37);
    let reports = scroll_stream(&sim, 5000, &mut r);
    assert!(reports.len() > 30_000, "only {} reports", reports.len());
    let t = std::time::Instant::now();
    let out = scroll::analyze_scroll(&reports, &scroll::ScrollConfig::default());
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    assert!(ms < 250.0, "analysis took {ms:.0} ms on {} reports", reports.len());
    assert_eq!(out.quantum, 8.0);
}

#[test]
fn scroll_refuses_a_capture_whose_clock_went_backwards() {
    let sim = ScrollSim::default();
    let mut r = rng(38);
    let mut reports = scroll_stream(&sim, 60, &mut r);
    let mid = reports.len() / 2;
    reports[mid].t_ns = reports[mid].t_ns.saturating_sub(500_000_000);
    let out = scroll::analyze_scroll(&reports, &scroll::ScrollConfig::default());
    assert_eq!(out.verdict, Verdict::Inconclusive);
}

// ---------------------------------------------------------------- lift-off

/// A shuttle across the desk, optionally with a band the sensor cannot see.
///
/// Blanking removes the reports outright rather than zeroing them, because that
/// is what a device does when it has nothing to send: a zeroed report would
/// still be a report, and would give the detector information the real failure
/// never provides.
fn shuttle(seed: u64, n_half: usize, blind: Option<(f64, f64)>) -> Vec<Report> {
    shuttle_full(seed, n_half, blind, 0.0, 1)
}

/// `blind_every` of 1 blinds every pass; 2 blinds every other PAIR of passes,
/// which is what a height near the threshold looks like. Pairs rather than
/// single passes because passes alternate direction, and blinding every other
/// one would blind only one direction, which the detector rightly refuses as a
/// habit rather than an obstacle.
fn shuttle_full(
    seed: u64,
    n_half: usize,
    blind: Option<(f64, f64)>,
    jitter_in: f64,
    blind_every: usize,
) -> Vec<Report> {
    let sim = MouseSim { cpi: 1600.0, ..Default::default() };
    let mut r = rng(seed);
    let mut traj = ShuttleTraj::new(&mut r, 4.0, 0.5, n_half);
    traj.end_jitter_in = jitter_in;
    let reports = sim.render(&traj, &mut r);
    let Some((centre_in, width_in)) = blind else {
        return reports;
    };
    // Dead-reckon along the sweep axis and drop what falls in the band.
    let mut x = 0.0f64;
    let mut out = Vec::with_capacity(reports.len());
    let mut pass = 0usize;
    let mut last_dir = 0i32;
    for rep in reports {
        let dir = if rep.dx >= 0 { 1 } else { -1 };
        if dir != last_dir {
            last_dir = dir;
            pass += 1;
        }
        x += rep.dx as f64 / 1600.0;
        if (pass / 2) % blind_every == 0 && (x - centre_in).abs() <= width_in / 2.0 {
            continue;
        }
        out.push(rep);
    }
    out
}

fn lod_cfg() -> lod::LodConfig {
    // 6 mm slot, which is 0.236 in.
    lod::LodConfig::new(1.0, 6.0, 1600.0)
}

#[test]
fn lod_a_mouse_that_keeps_the_surface_is_not_called_a_lift_off() {
    // The direction that matters most. The shuttle stops dead at both ends and
    // the device sends nothing while it is stopped, so this capture is FULL of
    // silences. Every one of them is a hand stopping, and not one may be
    // reported as the sensor letting go.
    let out = lod::analyze_lod(&shuttle(11, 30, None), &lod_cfg());
    assert_eq!(
        out.state,
        lod::LodState::Tracked,
        "clean sweep called {:?}: {}",
        out.state,
        out.note
    );
    assert_eq!(
        out.n_crossings, 0,
        "a turnaround was counted as a crossing {} time(s)",
        out.n_crossings
    );
}

#[test]
fn lod_a_slot_the_sensor_cannot_see_is_found() {
    let out = lod::analyze_lod(&shuttle(12, 30, Some((2.0, 0.236))), &lod_cfg());
    assert_eq!(
        out.state,
        lod::LodState::Lost,
        "a blind slot was not found: {}",
        out.note
    );
    // Two inches into the sweep is 50.8 mm. The two directions meet opposite
    // edges of the slot and the figure is their midpoint, so it lands on the
    // centre.
    assert!(
        (out.slot_at_mm - 50.8).abs() < 4.0,
        "slot located at {:.1} mm, expected about 50.8",
        out.slot_at_mm
    );
    assert!(
        out.slot_spread_mm < 3.0,
        "a fixed slot should not wander: spread was {:.1} mm",
        out.slot_spread_mm
    );
}

#[test]
fn lod_a_habit_of_pausing_is_not_a_lift_off() {
    // Silences entered and left at full speed, so the edge checks cannot see
    // anything wrong with them, but in a different place on every sweep. A slot
    // does not move between sweeps. This is the case the clustering exists for,
    // and without it a fidgety user would be reported as a mouse.
    let sim = MouseSim { cpi: 1600.0, ..Default::default() };
    let mut r = rng(13);
    let traj = ShuttleTraj::new(&mut r, 4.0, 0.5, 30);
    let reports = sim.render(&traj, &mut r);
    let mut x = 0.0f64;
    let mut stroke = 0usize;
    let mut last_dir = 0i32;
    let mut centre = 1.0f64;
    let mut out_r = Vec::with_capacity(reports.len());
    for rep in reports {
        let dir = if rep.dx >= 0 { 1 } else { -1 };
        if dir != last_dir {
            last_dir = dir;
            stroke += 1;
            // Wander the pause all over the stroke, as a person would.
            centre = 0.7 + 2.6 * ((stroke * 7919 % 100) as f64 / 100.0);
        }
        x += rep.dx as f64 / 1600.0;
        if (x - centre).abs() <= 0.236 / 2.0 {
            continue;
        }
        out_r.push(rep);
    }
    let out = lod::analyze_lod(&out_r, &lod_cfg());
    assert_ne!(
        out.state,
        lod::LodState::Lost,
        "wandering pauses were reported as a lift-off: {}",
        out.note
    );
}

#[test]
fn lod_a_disconnection_is_refused_rather_than_measured() {
    // The fault that produced this whole test. A mouse that drops off the bus
    // mid-run must never come back as a lift-off distance.
    let mut reports = shuttle(14, 30, None);
    let cut = reports.len() / 2;
    for rep in reports.iter_mut().skip(cut) {
        rep.t_ns += 2_000_000_000;
    }
    let out = lod::analyze_lod(&reports, &lod_cfg());
    assert_eq!(out.state, lod::LodState::Unknown);
    assert!(
        out.note.contains("disconnection"),
        "wrong refusal for a disconnect: {}",
        out.note
    );
}

#[test]
fn lod_too_little_sweeping_is_refused() {
    let out = lod::analyze_lod(&shuttle(15, 4, None), &lod_cfg());
    assert_eq!(out.state, lod::LodState::Unknown, "note was: {}", out.note);
}

#[test]
fn lod_the_ladder_refuses_without_a_control_and_brackets_with_one() {
    use lod::{summarize_lod, LodRung, LodState};
    let rungs = [
        LodRung { height_mm: 1.0, state: LodState::Tracked },
        LodRung { height_mm: 2.0, state: LodState::Lost },
    ];
    let no_control = summarize_lod(&rungs, None);
    assert_eq!(no_control.verdict, Verdict::Inconclusive);
    assert!(no_control.note.contains("control"), "{}", no_control.note);

    let with_control = [
        LodRung { height_mm: 0.0, state: LodState::Tracked },
        LodRung { height_mm: 1.0, state: LodState::Tracked },
        LodRung { height_mm: 2.0, state: LodState::Lost },
    ];
    let s = summarize_lod(&with_control, None);
    assert_eq!(s.verdict, Verdict::Pass, "{}", s.note);
    assert_eq!(s.tracked_to_mm, 1.0);
    assert_eq!(s.lost_at_mm, 2.0);
    assert_eq!(s.bracket_mm, 1.0);
}

#[test]
fn lod_a_ladder_that_contradicts_itself_is_refused() {
    use lod::{summarize_lod, LodRung, LodState};
    // A taller stack tracked than a shorter one did not: the rig moved.
    let rungs = [
        LodRung { height_mm: 0.0, state: LodState::Tracked },
        LodRung { height_mm: 1.0, state: LodState::Lost },
        LodRung { height_mm: 2.0, state: LodState::Tracked },
    ];
    let s = summarize_lod(&rungs, None);
    assert_eq!(s.verdict, Verdict::Inconclusive);
    assert!(s.note.contains("disagree"), "{}", s.note);
}

#[test]
fn lod_finds_a_slot_that_is_not_in_the_middle_of_the_sweep() {
    // The position assertion above cannot fail for a detector that reports the
    // midpoint of the sweep whatever the data, and pooling the two directions
    // made it do exactly that. A slot deliberately off centre pins it: 1.5
    // inches into a 4 inch sweep is 38.1 mm, a centimetre from the middle.
    let out = lod::analyze_lod(&shuttle(21, 30, Some((1.5, 0.236))), &lod_cfg());
    assert_eq!(out.state, lod::LodState::Lost, "off-centre slot: {}", out.note);
    assert!(
        (out.slot_at_mm - 38.1).abs() < 4.0,
        "slot located at {:.1} mm, expected about 38.1",
        out.slot_at_mm
    );
}

#[test]
fn lod_survives_a_hand_that_does_not_turn_in_the_same_place_twice() {
    // Measuring a crossing from the turn made the clustering a test of how
    // repeatably the user reverses. Nobody reverses to a few millimetres over
    // thirty sweeps, so a correct rig with a genuinely failing sensor was
    // refused. 5 mm of end wander is modest for a hand.
    let reports = shuttle_full(22, 30, Some((2.0, 0.236)), 5.0 / 25.4, 1);
    let out = lod::analyze_lod(&reports, &lod_cfg());
    assert_eq!(
        out.state,
        lod::LodState::Lost,
        "wandering turns broke the clustering: {} (spread {:.1} mm)",
        out.note,
        out.slot_spread_mm
    );
}

#[test]
fn lod_reports_a_height_near_the_threshold_as_marginal() {
    // Marginal is the state the ladder needs most, since it is what a height
    // just under the lift-off distance produces. Counting the turnaround
    // silences as suspicious stops made it unreachable at every blinding
    // fraction: the run had to lose more passes than it had turns.
    let reports = shuttle_full(23, 30, Some((2.0, 0.236)), 0.0, 2);
    let out = lod::analyze_lod(&reports, &lod_cfg());
    assert_eq!(
        out.state,
        lod::LodState::Marginal,
        "half-blinded run came out {:?}: {} (loss {:.2})",
        out.state,
        out.note,
        out.loss_fraction
    );
}

#[test]
fn lod_marginal_bounds_the_bracket_from_above() {
    use lod::{summarize_lod, LodRung, LodState};
    // A height that lost the pad on some passes is evidence the sensor was
    // already letting go there, so it cannot sit above a height reported as
    // tracked. Ignoring it let the summary claim tracking to 2.0 mm while the
    // table beside it showed 1.5 mm as marginal.
    let rungs = [
        LodRung { height_mm: 0.0, state: LodState::Tracked },
        LodRung { height_mm: 1.0, state: LodState::Tracked },
        LodRung { height_mm: 1.5, state: LodState::Marginal },
        LodRung { height_mm: 3.0, state: LodState::Lost },
    ];
    let s = summarize_lod(&rungs, None);
    assert_eq!(s.verdict, Verdict::Pass, "{}", s.note);
    assert_eq!(s.tracked_to_mm, 1.0);
    assert_eq!(s.lost_at_mm, 1.5, "marginal must bound the bracket");
}
