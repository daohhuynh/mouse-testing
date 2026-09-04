//! The session log: every captured event, written so it can be read back.
//!
//! FORMAT. One CSV file, with the metadata carried in `#` comment lines at the
//! top. That choice is deliberate: the raw event list is the large part, a
//! minute of a 1 kHz mouse is sixty thousand rows, and CSV is both the most
//! compact honest text format for that and the one every spreadsheet and
//! analysis tool already opens. Keeping the metadata in the same file rather
//! than a sidecar means a reload needs exactly one path, and `#` is the comment
//! character every common CSV reader already understands.
//!
//! WHAT IS RECORDED. Everything needed to run the whole analysis again on
//! another machine: per-event timestamps in nanoseconds, per-axis motion, both
//! wheels, and button transitions, each tagged with the level it came from. The
//! metadata carries the device identity, the host, and the clock's own
//! resolution and read cost, because an interval measurement means nothing
//! without knowing what measured it.
//!
//! WHAT IS NOT RECORDED. Nothing is written that was not captured from the
//! mouse under test. There is no key logging here, no window titles, no
//! cursor positions, and no clipboard.

use crate::core::debounce::ButtonEvent;
use std::fmt::Write as _;

/// One event as stored. Flat and stringly-typed on purpose: this is the file
/// format, and it should be readable without this program.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub t_ns: u64,
    /// "device", "system" or "app".
    pub level: Level,
    pub dx: i32,
    pub dy: i32,
    pub wheel: i32,
    pub hwheel: i32,
    /// 0 when this row is motion rather than a button transition.
    pub button: u8,
    /// Only meaningful when `button` is nonzero.
    pub down: bool,
    pub is_button: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Device,
    System,
    App,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Device => "device",
            Level::System => "system",
            Level::App => "app",
        }
    }

    pub const ALL: [Level; 3] = [Level::Device, Level::System, Level::App];

    pub fn parse(s: &str) -> Option<Level> {
        match s {
            "device" => Some(Level::Device),
            "system" => Some(Level::System),
            "app" => Some(Level::App),
            _ => None,
        }
    }
}

/// Everything about the run that is not an event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub device_name: String,
    pub device_ids: String,
    pub transport: String,
    pub os: String,
    pub arch: String,
    pub cpu: String,
    pub clock: String,
    pub clock_resolution_ns: f64,
    pub clock_cost_ns: f64,
    pub claimed_hz: String,
    pub claimed_cpi: String,
    /// Warnings the environment raised at capture time. Kept because a reload
    /// on a different machine must not silently lose the reason the original
    /// numbers were untrustworthy.
    pub warnings: Vec<String>,
    pub duration_s: f64,
    /// What each measurement in the battery concluded.
    ///
    /// The events alone cannot carry this. Most of the verdicts come from a
    /// protocol the user followed with their hands, and nothing in a list of
    /// motion reports says which ten seconds were the angle-snapping stroke.
    /// Recording them is what lets two configurations be compared rather than
    /// just two recordings.
    pub results: Vec<crate::core::battery::Record>,
    /// Measurements deliberately left out of the battery, by key.
    pub excluded: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SessionLog {
    pub meta: Meta,
    pub events: Vec<Event>,
}

const HEADER: &str = "t_ns,level,dx,dy,wheel,hwheel,button,edge";

fn escape(s: &str) -> String {
    // Newlines would break the one-comment-per-line rule the reader relies on.
    s.replace(['\n', '\r'], " ")
}

impl SessionLog {
    /// The complete file: metadata comments, then the header, then the events.
    pub fn to_csv(&self) -> String {
        let m = &self.meta;
        let mut s = String::with_capacity(64 * self.events.len() + 1024);
        s.push_str("# mouse-testing session log v1\n");
        for (k, v) in [
            ("device", m.device_name.as_str()),
            ("device_ids", m.device_ids.as_str()),
            ("transport", m.transport.as_str()),
            ("os", m.os.as_str()),
            ("arch", m.arch.as_str()),
            ("cpu", m.cpu.as_str()),
            ("clock", m.clock.as_str()),
            ("claimed_hz", m.claimed_hz.as_str()),
            ("claimed_cpi", m.claimed_cpi.as_str()),
        ] {
            let _ = writeln!(s, "# {k}: {}", escape(v));
        }
        let _ = writeln!(s, "# clock_resolution_ns: {}", m.clock_resolution_ns);
        let _ = writeln!(s, "# clock_cost_ns: {}", m.clock_cost_ns);
        let _ = writeln!(s, "# duration_s: {}", m.duration_s);
        for warning in &m.warnings {
            let _ = writeln!(s, "# warning: {}", escape(warning));
        }
        for r in &m.results {
            let _ = writeln!(s, "# result: {}", escape(&r.encode()));
        }
        for k in &m.excluded {
            let _ = writeln!(s, "# excluded: {}", escape(k));
        }
        s.push_str("# edge is 1 for press, 0 for release, and empty for motion rows\n");
        s.push_str(HEADER);
        s.push('\n');
        for e in &self.events {
            let _ = writeln!(
                s,
                "{},{},{},{},{},{},{},{}",
                e.t_ns,
                e.level.as_str(),
                e.dx,
                e.dy,
                e.wheel,
                e.hwheel,
                e.button,
                if e.is_button {
                    if e.down {
                        "1"
                    } else {
                        "0"
                    }
                } else {
                    ""
                }
            );
        }
        s
    }

    /// Read a log back. Unparseable rows are skipped and counted rather than
    /// aborting the load, because a truncated export from a crashed run is
    /// still worth most of what it contains; the count is returned so the
    /// interface can say how much was dropped instead of quietly losing it.
    pub fn from_csv(text: &str) -> Result<(SessionLog, usize), String> {
        let mut log = SessionLog::default();
        let mut skipped = 0usize;
        let mut saw_header = false;
        let mut first = true;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if first {
                first = false;
                if !line.starts_with("# mouse-testing session log") {
                    return Err(
                        "not a mouse-testing session log: the first line does not identify one"
                            .into(),
                    );
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("# ") {
                let (k, v) = match rest.split_once(": ") {
                    Some(kv) => kv,
                    None => continue,
                };
                let m = &mut log.meta;
                match k {
                    "device" => m.device_name = v.into(),
                    "device_ids" => m.device_ids = v.into(),
                    "transport" => m.transport = v.into(),
                    "os" => m.os = v.into(),
                    "arch" => m.arch = v.into(),
                    "cpu" => m.cpu = v.into(),
                    "clock" => m.clock = v.into(),
                    "claimed_hz" => m.claimed_hz = v.into(),
                    "claimed_cpi" => m.claimed_cpi = v.into(),
                    "clock_resolution_ns" => m.clock_resolution_ns = v.parse().unwrap_or(0.0),
                    "clock_cost_ns" => m.clock_cost_ns = v.parse().unwrap_or(0.0),
                    "duration_s" => m.duration_s = v.parse().unwrap_or(0.0),
                    "warning" => m.warnings.push(v.into()),
                    "excluded" => m.excluded.push(v.into()),
                    "result" => {
                        // A line this build cannot parse is skipped rather than
                        // failing the load: an export from a later version must
                        // still open here, minus what it does not understand.
                        if let Some(r) = crate::core::battery::Record::decode(v) {
                            m.results.push(r);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if !saw_header {
                if line != HEADER {
                    return Err(format!(
                        "unexpected column layout: found {line:?}, expected {HEADER:?}"
                    ));
                }
                saw_header = true;
                continue;
            }
            match parse_row(line) {
                Some(e) => log.events.push(e),
                None => skipped += 1,
            }
        }
        if !saw_header {
            return Err("the file has no event rows".into());
        }
        Ok((log, skipped))
    }

    /// Total path length in device counts, at one level.
    pub fn counts(&self, level: Level) -> f64 {
        self.motion(level).iter().map(|r| r.mag()).sum()
    }

    /// One level's motion, for re-running any of the sensor detectors over a
    /// loaded capture.
    pub fn motion(&self, level: Level) -> Vec<crate::core::sensor::Report> {
        self.events
            .iter()
            .filter(|e| e.level == level && !e.is_button)
            .map(|e| crate::core::sensor::Report {
                t_ns: e.t_ns,
                dx: e.dx,
                dy: e.dy,
                wheel: e.wheel,
                hwheel: e.hwheel,
            })
            .collect()
    }

    /// One level's reports, for re-running the interval analysis.
    pub fn reports(&self, level: Level) -> Vec<crate::core::polling::Report> {
        self.events
            .iter()
            .filter(|e| e.level == level && !e.is_button)
            .map(|e| crate::core::polling::Report {
                t_ns: e.t_ns,
                counts: e.dx.unsigned_abs().saturating_add(e.dy.unsigned_abs()) as i32,
            })
            .collect()
    }

    pub fn buttons(&self) -> Vec<ButtonEvent> {
        self.events
            .iter()
            .filter(|e| e.is_button)
            .map(|e| ButtonEvent {
                t_ns: e.t_ns,
                button: e.button,
                down: e.down,
            })
            .collect()
    }

    pub fn count(&self, level: Level) -> usize {
        self.events
            .iter()
            .filter(|e| e.level == level && !e.is_button)
            .count()
    }
}

fn parse_row(line: &str) -> Option<Event> {
    let mut f = line.split(',');
    let t_ns: u64 = f.next()?.parse().ok()?;
    let level = Level::parse(f.next()?)?;
    let dx: i32 = f.next()?.parse().ok()?;
    let dy: i32 = f.next()?.parse().ok()?;
    let wheel: i32 = f.next()?.parse().ok()?;
    let hwheel: i32 = f.next()?.parse().ok()?;
    let button: u8 = f.next()?.parse().ok()?;
    let edge = f.next()?;
    let (is_button, down) = match edge {
        "" => (false, false),
        "1" => (true, true),
        "0" => (true, false),
        _ => return None,
    };
    Some(Event {
        t_ns,
        level,
        dx,
        dy,
        wheel,
        hwheel,
        button,
        down,
        is_button,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionLog {
        SessionLog {
            meta: Meta {
                device_name: "Test Mouse, model 3".into(),
                device_ids: "0x1234 / 0x5678".into(),
                transport: "USB".into(),
                os: "macOS 15.6".into(),
                arch: "arm64".into(),
                cpu: "Apple M2".into(),
                clock: "mach_absolute_time".into(),
                clock_resolution_ns: 41.7,
                clock_cost_ns: 18.3,
                claimed_hz: "1000".into(),
                claimed_cpi: "1600".into(),
                warnings: vec!["running under a hypervisor".into()],
                duration_s: 12.5,
                excluded: vec!["sensor.lod".into()],
                results: vec![
                    crate::core::battery::Record {
                        key: "polling".into(),
                        verdict: Some(crate::core::sensor::Verdict::Pass),
                        headline: "1000.2 Hz nominal".into(),
                    },
                    crate::core::battery::Record {
                        key: "cps".into(),
                        verdict: None,
                        headline: "7.40 CPS sustained".into(),
                    },
                ],
            },
            events: vec![
                Event { t_ns: 1_000_000, level: Level::Device, dx: 3, dy: -2, wheel: 0,
                        hwheel: 0, button: 0, down: false, is_button: false },
                Event { t_ns: 2_000_000, level: Level::Device, dx: 0, dy: 0, wheel: 1,
                        hwheel: 0, button: 0, down: false, is_button: false },
                Event { t_ns: 3_000_000, level: Level::Device, dx: 0, dy: 0, wheel: 0,
                        hwheel: 0, button: 1, down: true, is_button: true },
                Event { t_ns: 3_500_000, level: Level::Device, dx: 0, dy: 0, wheel: 0,
                        hwheel: 0, button: 1, down: false, is_button: true },
                Event { t_ns: 4_000_000, level: Level::System, dx: -1, dy: 0, wheel: 0,
                        hwheel: 2, button: 0, down: false, is_button: false },
            ],
        }
    }

    #[test]
    fn a_log_survives_a_round_trip_unchanged() {
        let a = sample();
        let (b, skipped) = SessionLog::from_csv(&a.to_csv()).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(a.meta, b.meta, "metadata changed across the round trip");
        assert_eq!(a.events, b.events, "events changed across the round trip");
    }

    #[test]
    fn a_reloaded_log_can_be_analysed_again() {
        let (b, _) = SessionLog::from_csv(&sample().to_csv()).unwrap();
        assert_eq!(b.count(Level::Device), 2);
        assert_eq!(b.count(Level::System), 1);
        let m = b.motion(Level::Device);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].dx, m[0].dy), (3, -2));
        assert_eq!(m[1].wheel, 1);
        let btn = b.buttons();
        assert_eq!(btn.len(), 2);
        assert!(btn[0].down && !btn[1].down);
        assert_eq!(btn[0].button, 1);
    }

    #[test]
    fn the_warning_that_made_a_run_untrustworthy_survives_the_export() {
        // The whole point of recording these is that someone reading the file
        // on another machine cannot re-derive them.
        let (b, _) = SessionLog::from_csv(&sample().to_csv()).unwrap();
        assert_eq!(b.meta.warnings, vec!["running under a hypervisor".to_string()]);
        assert_eq!(b.meta.clock_resolution_ns, 41.7);
    }

    #[test]
    fn a_truncated_file_loads_what_it_has_and_says_what_it_lost() {
        let full = sample().to_csv();
        let mut text = full.clone();
        text.push_str("9999999,device,1,2\n"); // a row cut off mid-write
        text.push_str("garbage\n");
        let (b, skipped) = SessionLog::from_csv(&text).unwrap();
        assert_eq!(skipped, 2, "bad rows must be counted, not silently dropped");
        assert_eq!(b.events.len(), sample().events.len());
    }

    #[test]
    fn something_that_is_not_a_session_log_is_refused_by_name() {
        let err = SessionLog::from_csv("a,b,c\n1,2,3\n").unwrap_err();
        assert!(err.contains("not a mouse-testing session log"), "{err}");
        let err = SessionLog::from_csv("# mouse-testing session log v1\nx,y\n1,2\n").unwrap_err();
        assert!(err.contains("column layout"), "{err}");
    }

    #[test]
    fn a_field_containing_a_newline_cannot_break_the_comment_block() {
        let mut a = sample();
        a.meta.device_name = "Line one\nnot: a comment".into();
        let (b, skipped) = SessionLog::from_csv(&a.to_csv()).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(b.meta.device_name, "Line one not: a comment");
    }
}
