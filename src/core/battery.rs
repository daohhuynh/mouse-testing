//! The battery: which measurements count as part of one configuration, and
//! what each of them concluded.
//!
//! Everything in this app measures one thing at a time, and the question people
//! actually arrive with is bigger than any one of them: *I changed a setting on
//! the mouse, what did that do?* Answering it needs the whole set of results
//! from before the change beside the whole set from after.
//!
//! Two things follow, and they are why this module exists rather than the
//! comparison simply widening.
//!
//! The results have to survive being written down. A capture already exports
//! every event and reloads exactly, but the verdicts were only ever on screen,
//! so a comparison could reach the timing figures and nothing else. They are
//! recorded in the export now, one line each.
//!
//! And the set has to be chosen rather than assumed. A summary that says "not
//! measured this session" cannot tell a test the user never got to from one
//! they deliberately left out, and those mean opposite things when two runs are
//! put side by side: the first is a hole in the evidence and the second is the
//! shape of the experiment. Excluding a test is recorded as a decision.

use crate::app::App;
use crate::core::sensor::Verdict;

/// One measurement that can be part of a battery.
pub struct Item {
    pub key: &'static str,
    pub label: &'static str,
}

/// In the order they appear in the sidebar, so the list reads like the app.
pub const ITEMS: &[Item] = &[
    Item { key: "polling", label: "polling rate" },
    Item { key: "clicks", label: "click and debounce" },
    Item { key: "cps", label: "clicks per second" },
    Item { key: "ab", label: "A/B comparison" },
    Item { key: "sensor.cpi", label: "counts per inch" },
    Item { key: "sensor.drift", label: "drift and jitter" },
    Item { key: "sensor.snap", label: "angle snapping" },
    Item { key: "sensor.smooth", label: "motion smoothing" },
    Item { key: "sensor.tracking", label: "maximum tracking speed" },
    Item { key: "sensor.lod", label: "lift-off distance" },
    Item { key: "scroll.wheel", label: "scroll wheel" },
    Item { key: "scroll.tilt", label: "tilt wheel" },
];

pub fn label_for(key: &str) -> &str {
    ITEMS
        .iter()
        .find(|i| i.key == key)
        .map(|i| i.label)
        .unwrap_or(key)
}

/// One measurement's result, as recorded and as compared.
///
/// `verdict` is optional because not every measurement produces one. CPS and
/// A/B report numbers rather than judgements, and inventing a pass for them so
/// the type is tidy would put a verdict on screen that the app never made.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub key: String,
    pub verdict: Option<Verdict>,
    pub headline: String,
}

fn verdict_name(v: Verdict) -> &'static str {
    match v {
        Verdict::Pass => "pass",
        Verdict::Warn => "warn",
        Verdict::Fail => "fail",
        Verdict::Inconclusive => "inconclusive",
    }
}

fn verdict_from(s: &str) -> Option<Verdict> {
    match s {
        "pass" => Some(Verdict::Pass),
        "warn" => Some(Verdict::Warn),
        "fail" => Some(Verdict::Fail),
        "inconclusive" => Some(Verdict::Inconclusive),
        _ => None,
    }
}

impl Record {
    /// `key|verdict|headline`, one line, for the export's comment block.
    pub fn encode(&self) -> String {
        format!(
            "{}|{}|{}",
            self.key,
            self.verdict.map(verdict_name).unwrap_or("-"),
            self.headline.replace(['|', '\n', '\r'], " ")
        )
    }

    pub fn decode(line: &str) -> Option<Record> {
        let mut parts = line.splitn(3, '|');
        let key = parts.next()?.trim().to_string();
        let v = parts.next()?.trim();
        let headline = parts.next().unwrap_or("").trim().to_string();
        if key.is_empty() {
            return None;
        }
        Some(Record {
            key,
            verdict: verdict_from(v),
            headline,
        })
    }
}

/// What one measurement concluded, or None if it was never run.
pub fn measure(app: &App, key: &str) -> Option<Record> {
    let rec = |verdict: Option<Verdict>, headline: String| {
        Some(Record {
            key: key.to_string(),
            verdict,
            headline,
        })
    };
    match key {
        "polling" => {
            let r = &app.poll_result;
            if r.n_intervals == 0 {
                return None;
            }
            rec(
                Some(r.verdict),
                format!(
                    "{:.1} Hz nominal, {:.3}% dropped, {:.3}% late, {} intervals judged",
                    r.nominal_hz,
                    r.drop_rate * 100.0,
                    r.slow_rate * 100.0,
                    r.n_analyzable
                ),
            )
        }
        "clicks" => {
            let buttons = &app.session.buttons;
            if buttons.is_empty() {
                return None;
            }
            let mut ids: Vec<u8> = buttons.iter().map(|b| b.button).collect();
            ids.sort_unstable();
            ids.dedup();
            // The worst button, because a battery line has to be one line and
            // one bad switch is the finding whatever the others did.
            let mut worst = Verdict::Pass;
            let mut doublets = 0usize;
            for id in &ids {
                let evs: Vec<_> = buttons.iter().filter(|b| b.button == *id).copied().collect();
                let r = crate::core::debounce::analyze_button(
                    &evs,
                    *id,
                    &crate::core::debounce::DebounceConfig::default(),
                );
                worst = Verdict::worst(worst, r.verdict);
                doublets += r.n_doublets;
            }
            rec(
                Some(worst),
                format!(
                    "{} button(s), {} doublet(s), {} transitions",
                    ids.len(),
                    doublets,
                    buttons.len()
                ),
            )
        }
        "cps" => {
            let best = app
                .cps
                .history
                .iter()
                .map(|r| r.sustained_cps)
                .fold(f64::NEG_INFINITY, f64::max);
            if !best.is_finite() {
                return None;
            }
            rec(
                None,
                format!(
                    "{:.2} CPS sustained, best of {} run(s)",
                    best,
                    app.cps.history.len()
                ),
            )
        }
        "ab" => {
            let run = app.ab.run.as_ref()?;
            let r = run.analyse(0.05);
            let unit = run.plan.variant.unit();
            let shift = r.paired_shift.as_ref().unwrap_or(&r.unpaired_shift);
            rec(
                None,
                format!(
                    "{} vs {}: {:+.3} {unit}, p {:.4}",
                    run.plan.label_a, run.plan.label_b, shift.estimate, r.p_primary
                ),
            )
        }
        "sensor.cpi" => {
            let s = app.sensor.cpi_summary.as_ref()?;
            rec(
                Some(s.verdict),
                format!(
                    "{:.0} CPI measured, {:+.2}% off claimed, {} swipe(s)",
                    s.median_cpi,
                    s.deviation * 100.0,
                    s.n_trials
                ),
            )
        }
        "sensor.drift" => {
            let r = app.sensor.drift.as_ref()?;
            rec(
                Some(r.verdict),
                format!("{:.2} counts/s drift, {:.2} counts/s jitter", r.drift_cps, r.jitter_cps),
            )
        }
        "sensor.snap" => {
            let r = app.sensor.snap.as_ref()?;
            rec(Some(r.verdict), r.note.to_string())
        }
        "sensor.smooth" => {
            let r = app.sensor.smooth.as_ref()?;
            rec(Some(r.verdict), r.note.to_string())
        }
        "sensor.tracking" => {
            let r = app.sensor.tracking.as_ref()?;
            rec(
                Some(r.verdict),
                format!(
                    "{:.0} IPS{}",
                    r.max_tracking_ips,
                    if r.bounded_below { " (lower bound)" } else { "" }
                ),
            )
        }
        "sensor.lod" => {
            let s = app.sensor.lod_summary.as_ref()?;
            rec(
                Some(s.verdict),
                if s.verdict == Verdict::Inconclusive {
                    s.note.to_string()
                } else {
                    format!(
                        "between {:.2} and {:.2} mm ({:.2} mm bracket)",
                        s.tracked_to_mm, s.lost_at_mm, s.bracket_mm
                    )
                },
            )
        }
        "scroll.wheel" | "scroll.tilt" => {
            let r = if key == "scroll.wheel" {
                app.scroll.vertical.as_ref()?
            } else {
                app.scroll.horizontal.as_ref()?
            };
            rec(
                Some(r.verdict),
                format!(
                    "{} up, {} down, {} reversed, {} skipped",
                    r.detents_up, r.detents_down, r.reversals, r.skips
                ),
            )
        }
        _ => None,
    }
}

/// Every measurement the user has left in the battery and that has a result.
///
/// A measurement that is switched off contributes nothing, which is the point:
/// the export then records the shape of the experiment rather than everything
/// that happened to be on screen when it was saved.
pub fn snapshot(app: &App) -> Vec<Record> {
    ITEMS
        .iter()
        .filter(|i| !app.battery_off.contains(i.key))
        .filter_map(|i| measure(app, i.key))
        .collect()
}

/// One row of the comparison.
pub struct Row {
    pub key: String,
    pub before: Option<Record>,
    pub after: Option<Record>,
}

impl Row {
    /// Whether the verdict moved, which is the only thing worth colouring.
    pub fn changed(&self) -> bool {
        match (&self.before, &self.after) {
            (Some(a), Some(b)) => a.verdict != b.verdict,
            _ => false,
        }
    }
}

/// Pair two snapshots up by key, keeping anything either of them has.
///
/// A measurement one side lacks is kept and marked, never dropped. Dropping it
/// would turn "we did not measure this after the change" into silence, and
/// silence reads as agreement.
pub fn compare(before: &[Record], after: &[Record]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for item in ITEMS {
        let b = before.iter().find(|r| r.key == item.key).cloned();
        let a = after.iter().find(|r| r.key == item.key).cloned();
        if b.is_some() || a.is_some() {
            rows.push(Row {
                key: item.key.to_string(),
                before: b,
                after: a,
            });
        }
    }
    // Anything from an older or newer version of the app that this build does
    // not know about is still shown, rather than quietly disappearing.
    for r in before.iter().chain(after) {
        if ITEMS.iter().any(|i| i.key == r.key) || rows.iter().any(|x| x.key == r.key) {
            continue;
        }
        rows.push(Row {
            key: r.key.clone(),
            before: before.iter().find(|x| x.key == r.key).cloned(),
            after: after.iter().find(|x| x.key == r.key).cloned(),
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_survives_the_round_trip_including_the_awkward_parts() {
        for r in [
            Record { key: "polling".into(), verdict: Some(Verdict::Pass), headline: "1000.2 Hz".into() },
            Record { key: "cps".into(), verdict: None, headline: "7.40 CPS".into() },
            Record { key: "sensor.lod".into(), verdict: Some(Verdict::Inconclusive), headline: String::new() },
        ] {
            let back = Record::decode(&r.encode()).expect("decode");
            assert_eq!(back, r);
        }
    }

    #[test]
    fn a_headline_cannot_break_the_line_format() {
        // The separator and newlines both have to survive being in the text,
        // because a headline is built from a note the user never chose.
        let r = Record {
            key: "ab".into(),
            verdict: None,
            headline: "a|b\nc".into(),
        };
        let line = r.encode();
        assert_eq!(line.matches('|').count(), 2);
        assert!(!line.contains('\n'));
        let back = Record::decode(&line).expect("decode");
        assert_eq!(back.key, "ab");
        assert_eq!(back.verdict, None);
    }

    #[test]
    fn a_missing_measurement_is_kept_and_marked_rather_than_dropped() {
        // Dropping it would turn "not measured after the change" into silence,
        // and silence in a comparison reads as agreement.
        let before = vec![
            Record { key: "polling".into(), verdict: Some(Verdict::Pass), headline: "a".into() },
            Record { key: "sensor.cpi".into(), verdict: Some(Verdict::Warn), headline: "b".into() },
        ];
        let after = vec![Record {
            key: "polling".into(),
            verdict: Some(Verdict::Fail),
            headline: "c".into(),
        }];
        let rows = compare(&before, &after);
        assert_eq!(rows.len(), 2);
        let polling = rows.iter().find(|r| r.key == "polling").unwrap();
        assert!(polling.changed(), "pass to fail must register as a change");
        let cpi = rows.iter().find(|r| r.key == "sensor.cpi").unwrap();
        assert!(cpi.after.is_none(), "the after side should be absent");
        assert!(!cpi.changed(), "a missing side is not a changed verdict");
    }

    #[test]
    fn a_key_this_build_does_not_know_still_appears() {
        // An export from a later version must not quietly lose rows.
        let before = vec![Record {
            key: "sensor.future".into(),
            verdict: Some(Verdict::Pass),
            headline: "x".into(),
        }];
        let rows = compare(&before, &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, "sensor.future");
    }
}
