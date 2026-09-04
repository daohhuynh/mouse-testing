//! What each sensor test asks the user to do, and how long it listens for.
//!
//! Every one of these tests measures something the mouse only reveals under a
//! specific hand movement, and the detector behind it will refuse rather than
//! guess when the movement was wrong. Keeping the instructions next to the
//! capture length means the screen and the analysis cannot drift apart.

/// The six sensor measurements, in the order they appear in the interface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Test {
    Cpi,
    Drift,
    Snap,
    Smooth,
    Tracking,
    Lod,
}

impl Test {
    pub const ALL: [Test; 6] = [
        Test::Cpi,
        Test::Drift,
        Test::Snap,
        Test::Smooth,
        Test::Tracking,
        Test::Lod,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Test::Cpi => "counts per inch",
            Test::Drift => "drift and jitter while stationary",
            Test::Snap => "angle snapping and path correction",
            Test::Smooth => "motion smoothing",
            Test::Tracking => "maximum tracking speed",
            Test::Lod => "lift-off distance",
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
            Test::Lod => "How high can it rise before the sensor stops seeing the pad?",
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
            // Thirty to forty half-strokes, so a dozen clean crossings survive
            // discarding the ones whose edges fall awkwardly.
            Test::Lod => 20.0,
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
            Test::Lod => &[
                "Build the runway. Two piles of the same number of cards, side by side on \
                 the pad, with a slot of a few millimetres between them running across the \
                 way you will sweep. Tape them down. The mouse rides on the cards and the \
                 sensor looks down the slot at the pad, so over the cards it sits at its \
                 normal height and over the slot it is raised by the thickness of one pile. \
                 A stack laid flat UNDER the whole mouse raises the sensor and the surface \
                 it looks at by the same amount and measures nothing.",
                "The mouse never leaves the desk in this test. Nothing is lifted, so a \
                 cable never moves.",
                "Check the slot by sliding the mouse across it slowly. If a foot drops in, \
                 the slot is too wide: the mouse tilts and the height stops being the \
                 height you measured. Make it narrower.",
                "Measure twenty cards in one stack with a ruler and enter that figure, then \
                 how many cards are in each pile and how wide the slot is. Twenty at once \
                 because a card is only about a third of a millimetre, so a ruler read to \
                 half a millimetre cannot resolve one; dividing that reading across twenty \
                 costs one measurement and buys back a factor of twenty.",
                "Run the control first, with the cards taken away and 0 entered. It says \
                 nothing about the mouse: it proves that you sweep without stopping and \
                 that the link is not dropping reports. Nothing at any height is judged \
                 until it passes.",
                "Press start, wait out the countdown, then sweep back and forth across the \
                 slot for the whole recording. Turn round at both ends, well clear of the \
                 slot, and do not stop in the middle of a sweep. The turns are not wasted \
                 time: each one is a full stop taken with the sensor on the surface, and \
                 that is how the app learns what your own stops look like instead of \
                 assuming.",
                "Then change the number of cards and run it again. Each run answers one \
                 question, did it track at that height, and the answer is the gap between \
                 the tallest pile that tracked and the shortest that did not. That gap can \
                 never be narrower than one card.",
            ],
        }
    }
}
