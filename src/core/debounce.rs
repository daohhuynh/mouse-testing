//! Button behaviour: press and release accounting, and contact bounce.
//!
//! Contact bounce is one physical press registering as several clicks. The
//! mechanical signature is well bounded: mouse microswitches settle within
//! single-digit milliseconds, which is why firmware debounce windows are
//! typically eight to fifteen. A human cannot approach that. Ordinary
//! double-clicks land 100 to 250 ms apart, and even a trained click-spammer at
//! twenty clicks per second leaves twenty to forty-five milliseconds between
//! releasing and pressing again.
//!
//! So a release-to-press gap under fifteen milliseconds is below anything a
//! hand can do and above where a healthy switch has finished settling. Combined
//! with a press that is itself only a few milliseconds long, it is the classic
//! worn-switch doublet and nothing else.
//!
//! The one trap is that spam-clicking compresses gaps into the suspect band
//! legitimately, so when the median gap shows the user was spam-clicking rather
//! than making deliberate presses, the suspect band reports inconclusive
//! instead of accusing a healthy mouse.

use crate::core::polling::Verdict;

#[derive(Clone, Copy, Debug)]
pub struct ButtonEvent {
    pub t_ns: u64,
    /// Raw identifier from the device, not a mapped name, so an unexpected
    /// button is visible rather than silently dropped.
    pub button: u8,
    pub down: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DebounceConfig {
    /// Below this, a release-to-press gap is beyond human capability.
    pub bounce_fail_ms: f64,
    /// Between fail and this, a gap is suspicious but reachable by a fast hand.
    pub bounce_warn_ms: f64,
    pub dwell_fail_ms: f64,
    pub dwell_warn_ms: f64,
    /// Proportion of presses that must look like bounce before failing.
    pub rate_fail: f64,
    /// If the median gap is below this the user was spam-clicking, and the
    /// suspect band stops meaning anything.
    pub min_median_gap_ms: f64,
    /// Below this many presses, detection power is too low to conclude.
    pub min_presses: usize,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        DebounceConfig {
            bounce_fail_ms: 15.0,
            bounce_warn_ms: 35.0,
            dwell_fail_ms: 15.0,
            dwell_warn_ms: 30.0,
            rate_fail: 0.01,
            min_median_gap_ms: 100.0,
            min_presses: 20,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub down_ns: u64,
    pub up_ns: Option<u64>,
    /// How long the button was held, milliseconds.
    pub dwell_ms: f64,
    /// Time from the previous release to this press, milliseconds.
    pub gap_ms: f64,
}

#[derive(Clone, Debug, Default)]
pub struct ButtonResult {
    pub button: u8,
    pub verdict: Verdict,
    pub n_down: usize,
    pub n_up: usize,
    /// Presses with no matching release, and releases with no matching press.
    pub unmatched_down: usize,
    pub unmatched_up: usize,
    /// The user was clicking too fast for the suspect band to mean anything.
    pub spam_clicking: bool,
    pub n_presses: usize,
    pub n_bounce_fail: usize,
    pub n_bounce_warn: usize,
    /// Gap and dwell both implausibly short: the definitive worn-switch shape.
    pub n_doublets: usize,
    pub n_short_dwell_fail: usize,
    pub n_short_dwell_warn: usize,
    pub bounce_rate: f64,
    pub min_gap_ms: f64,
    pub median_gap_ms: f64,
    pub min_dwell_ms: f64,
    pub median_dwell_ms: f64,
    /// Gap below which a press counts as a doublet, after scaling by the
    /// clicker's own median.
    pub doublet_gap_ms: f64,
    pub dwell_ms: Vec<f64>,
    pub gap_ms: Vec<f64>,
    pub note: &'static str,
}

fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        0.5 * (s[n / 2 - 1] + s[n / 2])
    }
}

/// Presses for one button, in time order.
///
/// Repeated edges in the same direction are ignored rather than treated as new
/// presses: two downs with no release between them is a lost release, which is
/// counted, not a second press.
pub fn presses_for(events: &[ButtonEvent], button: u8) -> (Vec<Press>, usize, usize) {
    let mut evs: Vec<ButtonEvent> = events.iter().copied().filter(|e| e.button == button).collect();
    evs.sort_by_key(|e| e.t_ns);

    let mut presses: Vec<Press> = Vec::new();
    let mut unmatched_down = 0usize;
    let mut unmatched_up = 0usize;
    let mut open: Option<u64> = None;
    let mut last_up: Option<u64> = None;

    for e in &evs {
        if e.down {
            if open.is_some() {
                // A press that was never released before this one.
                unmatched_down += 1;
                continue;
            }
            open = Some(e.t_ns);
        } else {
            match open.take() {
                Some(down) => {
                    let dwell_ms = (e.t_ns.saturating_sub(down)) as f64 / 1e6;
                    let gap_ms = match last_up {
                        Some(u) if down > u => (down - u) as f64 / 1e6,
                        _ => f64::NAN,
                    };
                    presses.push(Press {
                        down_ns: down,
                        up_ns: Some(e.t_ns),
                        dwell_ms,
                        gap_ms,
                    });
                    last_up = Some(e.t_ns);
                }
                None => unmatched_up += 1,
            }
        }
    }
    if let Some(down) = open {
        // Still held when the run ended.
        presses.push(Press {
            down_ns: down,
            up_ns: None,
            dwell_ms: f64::NAN,
            gap_ms: match last_up {
                Some(u) if down > u => (down - u) as f64 / 1e6,
                _ => f64::NAN,
            },
        });
    }
    (presses, unmatched_down, unmatched_up)
}

pub fn analyze_button(events: &[ButtonEvent], button: u8, cfg: &DebounceConfig) -> ButtonResult {
    let (presses, unmatched_down, unmatched_up) = presses_for(events, button);
    let mut r = ButtonResult {
        button,
        n_down: events.iter().filter(|e| e.button == button && e.down).count(),
        n_up: events.iter().filter(|e| e.button == button && !e.down).count(),
        unmatched_down,
        unmatched_up,
        n_presses: presses.len(),
        ..Default::default()
    };

    r.dwell_ms = presses.iter().map(|p| p.dwell_ms).filter(|d| d.is_finite()).collect();
    r.gap_ms = presses.iter().map(|p| p.gap_ms).filter(|g| g.is_finite()).collect();
    r.min_dwell_ms = r.dwell_ms.iter().copied().fold(f64::INFINITY, f64::min);
    r.min_gap_ms = r.gap_ms.iter().copied().fold(f64::INFINITY, f64::min);
    r.median_dwell_ms = median(&r.dwell_ms);
    r.median_gap_ms = median(&r.gap_ms);

    r.spam_clicking = r.median_gap_ms.is_finite() && r.median_gap_ms < cfg.min_median_gap_ms;

    // A doublet is convincing because it is an outlier against the clicker's
    // own rhythm, not merely because it is short in absolute terms. Someone
    // clicking twelve times a second has gaps near forty milliseconds, and a
    // fixed fifteen millisecond rule sits close enough to that to fire on a
    // healthy switch. Scaling by the observed median keeps the rule definitive
    // for a deliberate clicker and honest for a fast one.
    //
    // The scaling needs a rhythm to scale against. Below ten gaps there is no
    // median worth the name, and with only a couple of presses the median can
    // be the bounce itself, so the absolute rule stands alone there.
    const MIN_FOR_RHYTHM: usize = 10;
    let have_rhythm = r.gap_ms.len() >= MIN_FOR_RHYTHM;
    let doublet_gap_ms = if have_rhythm && r.median_gap_ms.is_finite() {
        cfg.bounce_fail_ms.min(r.median_gap_ms / 5.0)
    } else {
        cfg.bounce_fail_ms
    };
    let doublet_dwell_ms = if have_rhythm && r.median_dwell_ms.is_finite() {
        cfg.dwell_fail_ms.min(r.median_dwell_ms / 3.0)
    } else {
        cfg.dwell_fail_ms
    };

    for p in &presses {
        if p.gap_ms.is_finite() {
            if p.gap_ms < cfg.bounce_fail_ms {
                r.n_bounce_fail += 1;
                if p.dwell_ms.is_finite()
                    && p.gap_ms < doublet_gap_ms
                    && p.dwell_ms < doublet_dwell_ms
                {
                    r.n_doublets += 1;
                }
            } else if p.gap_ms < cfg.bounce_warn_ms {
                r.n_bounce_warn += 1;
            }
        }
        if p.dwell_ms.is_finite() {
            if p.dwell_ms < cfg.dwell_fail_ms {
                r.n_short_dwell_fail += 1;
            } else if p.dwell_ms < cfg.dwell_warn_ms {
                r.n_short_dwell_warn += 1;
            }
        }
    }
    r.doublet_gap_ms = doublet_gap_ms;
    r.bounce_rate = if r.n_presses > 0 {
        r.n_bounce_fail as f64 / r.n_presses as f64
    } else {
        0.0
    };

    r.verdict = verdict(&r, cfg);
    r.note = note_for(&r, cfg);
    r
}

fn verdict(r: &ButtonResult, cfg: &DebounceConfig) -> Verdict {
    // A doublet is definitive whatever else is going on: no hand can press
    // within fifteen milliseconds of releasing and hold for under fifteen more.
    if r.n_doublets > 0 {
        return Verdict::Fail;
    }
    if r.n_presses < cfg.min_presses {
        return Verdict::Inconclusive;
    }
    if r.bounce_rate >= cfg.rate_fail {
        return Verdict::Fail;
    }
    if r.unmatched_down > 0 || r.unmatched_up > 0 {
        return Verdict::Warn;
    }
    if r.n_bounce_fail > 0 {
        return Verdict::Warn;
    }
    if r.spam_clicking {
        // The suspect band cannot separate a fast hand from a marginal switch.
        return if r.n_bounce_warn > 0 {
            Verdict::Inconclusive
        } else {
            Verdict::Pass
        };
    }
    if r.n_bounce_warn > 0 || r.n_short_dwell_fail > 0 {
        return Verdict::Warn;
    }
    Verdict::Pass
}

fn note_for(r: &ButtonResult, cfg: &DebounceConfig) -> &'static str {
    if r.n_doublets > 0 {
        return "A press was registered within fifteen milliseconds of the previous release \
                and held for less than fifteen milliseconds. No hand can do that. This is \
                the signature of a worn switch registering one physical press twice.";
    }
    if r.n_presses < cfg.min_presses {
        return "Too few presses to conclude anything. Contact bounce is intermittent: \
                catching a switch that misfires two percent of the time needs about a \
                hundred presses for a reasonable chance, and two hundred to be confident.";
    }
    if r.spam_clicking && r.n_bounce_warn > 0 {
        return "You were clicking fast enough that the gaps between clicks overlap the \
                range a marginal switch produces, so these cannot be told apart. Click \
                deliberately, about once a second, and run it again.";
    }
    if r.unmatched_down > 0 || r.unmatched_up > 0 {
        return "Presses and releases did not pair up. Either an event was lost, or the \
                button was already held when the run started.";
    }
    ""
}

pub fn analyze_all(events: &[ButtonEvent], cfg: &DebounceConfig) -> Vec<ButtonResult> {
    let mut buttons: Vec<u8> = events.iter().map(|e| e.button).collect();
    buttons.sort_unstable();
    buttons.dedup();
    buttons
        .into_iter()
        .map(|b| analyze_button(events, b, cfg))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::sim::Rng;

    /// Generates a click stream with a known bounce rate.
    fn clicks(
        n: usize,
        mean_gap_ms: f64,
        mean_dwell_ms: f64,
        bounce_prob: f64,
        rng: &mut Rng,
    ) -> (Vec<ButtonEvent>, usize) {
        let mut evs = Vec::new();
        let mut t = 1_000_000_000u64;
        let mut bounces = 0;
        for _ in 0..n {
            // Lognormal, not gaussian with a floor: human inter-click and hold
            // times are strictly positive and right-skewed, and a clamped
            // gaussian manufactures impossibly short presses at the tail that
            // no hand produces.
            let gap = mean_gap_ms * (0.25 * rng.normal()).exp();
            t += (gap * 1e6) as u64;
            let dwell = mean_dwell_ms * (0.25 * rng.normal()).exp();
            evs.push(ButtonEvent { t_ns: t, button: 1, down: true });
            t += (dwell * 1e6) as u64;
            evs.push(ButtonEvent { t_ns: t, button: 1, down: false });

            if rng.unit() < bounce_prob {
                // A worn switch re-closes a few milliseconds after opening, for
                // a few milliseconds.
                bounces += 1;
                let bgap = 1.0 + 8.0 * rng.unit();
                let bdwell = 1.0 + 6.0 * rng.unit();
                t += (bgap * 1e6) as u64;
                evs.push(ButtonEvent { t_ns: t, button: 1, down: true });
                t += (bdwell * 1e6) as u64;
                evs.push(ButtonEvent { t_ns: t, button: 1, down: false });
            }
        }
        (evs, bounces)
    }

    #[test]
    fn healthy_deliberate_clicking_passes() {
        let cfg = DebounceConfig::default();
        for seed in 0..12u64 {
            let mut rng = Rng::new(100 + seed);
            let (evs, _) = clicks(60, 600.0, 90.0, 0.0, &mut rng);
            let r = analyze_button(&evs, 1, &cfg);
            assert_eq!(
                r.verdict,
                Verdict::Pass,
                "seed {seed}: healthy clicking judged {:?} ({})",
                r.verdict,
                r.note
            );
            assert_eq!(r.n_doublets, 0);
            assert_eq!(r.n_presses, 60);
            assert_eq!(r.n_down, 60);
            assert_eq!(r.n_up, 60);
        }
    }

    #[test]
    fn a_worn_switch_fails() {
        let cfg = DebounceConfig::default();
        for (rate, seed) in [(0.10f64, 1u64), (0.30, 2), (0.02, 3)] {
            let mut rng = Rng::new(200 + seed);
            let (evs, injected) = clicks(200, 500.0, 90.0, rate, &mut rng);
            let r = analyze_button(&evs, 1, &cfg);
            assert_eq!(
                r.verdict,
                Verdict::Fail,
                "a switch bouncing {:.0}% of the time was judged {:?}",
                rate * 100.0,
                r.verdict
            );
            assert!(
                r.n_doublets > 0,
                "{injected} bounces injected but none detected as doublets"
            );
        }
    }

    #[test]
    fn fast_spam_clicking_is_not_accused() {
        // Twelve clicks a second: gaps land squarely in the suspect band, which
        // is what made this the classic false positive.
        let cfg = DebounceConfig::default();
        let mut fails = 0;
        for seed in 0..40u64 {
            let mut rng = Rng::new(300 + seed);
            let (evs, _) = clicks(60, 45.0, 35.0, 0.0, &mut rng);
            let r = analyze_button(&evs, 1, &cfg);
            assert!(r.spam_clicking, "seed {seed}: spam guard did not engage");
            if r.verdict == Verdict::Fail {
                fails += 1;
            }
        }
        assert_eq!(fails, 0, "{fails} of 40 healthy spam-clicking runs were failed");
    }

    #[test]
    fn a_doublet_fails_even_in_a_short_run() {
        let cfg = DebounceConfig::default();
        let evs = vec![
            ButtonEvent { t_ns: 0, button: 1, down: true },
            ButtonEvent { t_ns: 80_000_000, button: 1, down: false },
            // Re-closes 4 ms later for 3 ms: mechanically impossible for a hand.
            ButtonEvent { t_ns: 84_000_000, button: 1, down: true },
            ButtonEvent { t_ns: 87_000_000, button: 1, down: false },
        ];
        let r = analyze_button(&evs, 1, &cfg);
        assert_eq!(r.verdict, Verdict::Fail);
        assert_eq!(r.n_doublets, 1);
    }

    #[test]
    fn too_few_presses_is_inconclusive_not_a_pass() {
        let cfg = DebounceConfig::default();
        let mut rng = Rng::new(7);
        let (evs, _) = clicks(5, 600.0, 90.0, 0.0, &mut rng);
        let r = analyze_button(&evs, 1, &cfg);
        assert_eq!(r.verdict, Verdict::Inconclusive);
        assert!(r.note.contains("Too few presses"));
    }

    #[test]
    fn unpaired_edges_are_counted_not_dropped() {
        let evs = vec![
            ButtonEvent { t_ns: 0, button: 1, down: true },
            // Second press with no release between.
            ButtonEvent { t_ns: 10_000_000, button: 1, down: true },
            ButtonEvent { t_ns: 90_000_000, button: 1, down: false },
            // Release with no press.
            ButtonEvent { t_ns: 95_000_000, button: 1, down: false },
        ];
        let r = analyze_button(&evs, 1, &DebounceConfig::default());
        assert_eq!(r.unmatched_down, 1);
        assert_eq!(r.unmatched_up, 1);
        assert_eq!(r.n_down, 2);
        assert_eq!(r.n_up, 2);
        assert_eq!(r.n_presses, 1);
    }

    #[test]
    fn every_button_is_reported_separately_by_raw_id() {
        let mut evs = Vec::new();
        for (b, base) in [(1u8, 0u64), (2, 1_000_000_000), (9, 2_000_000_000)] {
            evs.push(ButtonEvent { t_ns: base, button: b, down: true });
            evs.push(ButtonEvent { t_ns: base + 80_000_000, button: b, down: false });
        }
        let all = analyze_all(&evs, &DebounceConfig::default());
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().map(|r| r.button).collect::<Vec<_>>(), vec![1, 2, 9]);
    }
}
