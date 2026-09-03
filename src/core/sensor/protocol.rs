//! What each sensor test asks the user to do, and how long it listens for.
//!
//! Every one of these tests measures something the mouse only reveals under a
//! specific hand movement, and the detector behind it will refuse rather than
//! guess when the movement was wrong. Keeping the instructions next to the
//! capture length means the screen and the analysis cannot drift apart.

/// The five sensor measurements, in the order they appear in the interface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Test {
    Cpi,
    Drift,
    Snap,
    Smooth,
    Tracking,
}

impl Test {
    pub const ALL: [Test; 5] = [
        Test::Cpi,
        Test::Drift,
        Test::Snap,
        Test::Smooth,
        Test::Tracking,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Test::Cpi => "counts per inch",
            Test::Drift => "drift and jitter while stationary",
            Test::Snap => "angle snapping and path correction",
            Test::Smooth => "motion smoothing",
            Test::Tracking => "maximum tracking speed",
        }
    }

    /// What the test is for, in one line.
    pub fn purpose(self) -> &'static str {
        match self {
            Test::Cpi => "Does the mouse actually count what it claims to count?",
            Test::Drift => "Does the pointer move when your hand does not?",
            Test::Snap => "Does the firmware straighten your line for you?",
            Test::Smooth => "Does motion keep arriving after the mouse has stopped?",
            Test::Tracking => "How fast can you move before the sensor loses the surface?",
        }
    }

    /// Seconds of capture. Each is set by what the detector needs, not by feel.
    pub fn capture_s(self) -> f64 {
        match self {
            // One swipe plus room to line the mouse up and stop cleanly.
            Test::Cpi => 6.0,
            // The drift detector's own minimum is 10 s; 15 s gives the
            // self-normalised mean enough samples to separate a small bias from
            // a random walk.
            Test::Drift => 15.0,
            // One long stroke. 64 reports is the floor, and 2 inches of travel,
            // so a slow careful line still fits.
            Test::Snap => 8.0,
            // A glide and a stop, with time to set up the stop surface.
            Test::Smooth => 8.0,
            // Several swipes of increasing speed.
            Test::Tracking => 20.0,
        }
    }

    /// The instructions, one step per line.
    pub fn steps(self) -> &'static [&'static str] {
        match self {
            Test::Cpi => &[
                "Put two marks on the desk a measured distance apart, along a straight edge \
                 if you have one. Longer is better: 8 inches beats 2.",
                "Enter that distance and the CPI the mouse is configured to, above.",
                "Line the mouse up on the first mark, press start, then wait for the \
                 countdown to finish.",
                "Slide the mouse in one steady straight movement to the second mark and \
                 stop. Do not lift it, and do not go fast: above 40 inches per second the \
                 result is thrown out rather than reported.",
                "Repeat at least three times. The result is the median across runs, and the \
                 spread between them is the honest measure of how repeatable this is.",
            ],
            Test::Drift => &[
                "Put the mouse down on the surface you normally use and take your hand off \
                 it completely.",
                "Press start and do not touch the mouse, the desk, or the cable for the \
                 whole capture.",
                "A mouse that reports nothing at all here is the ideal result, not a \
                 failure of the test.",
            ],
            Test::Snap => &[
                "Aim for a long straight line, at least 2 inches, at a normal speed. Slower \
                 than 5 inches per second and the result is thrown out, because a careful \
                 hand and a correcting firmware look the same at a creep.",
                "Press start, wait for the countdown, then draw the line in one movement.",
                "Draw it at an angle, not along the edge of the desk. A line you drew \
                 perfectly horizontally cannot be told from one the firmware snapped to \
                 horizontal.",
                "This test needs the sensor's own noise to work with. On a very quiet \
                 sensor it will say so rather than pass you.",
            ],
            Test::Smooth => &[
                "Put a book or the edge of the mousepad down as something to stop against.",
                "Press start, wait for the countdown, then glide at a steady moderate speed \
                 into the stop so the mouse halts in one report.",
                "The abrupt stop is the whole point. A hand slowing down on its own looks \
                 exactly like a filter, and no statistic can separate them; a wall can.",
                "Moderate means roughly 15 to 25 inches per second. Faster than that and \
                 the result is thrown out.",
            ],
            Test::Tracking => &[
                "Give yourself as much clear desk as you can.",
                "Press start, then make repeated swipes, each one faster than the last, \
                 until you are moving as fast as you physically can.",
                "Keep the mouse flat on the surface. A lifted mouse is a different \
                 measurement.",
                "If nothing ever breaks, the answer is a lower bound: you did not find the \
                 limit, which is not the same as there being none.",
            ],
        }
    }
}
