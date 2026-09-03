// The icon's geometry. Included by icon.rs; see the header there.
//
// Coordinates are the unit square, origin top-left. The tile follows Apple's
// macOS icon grid: on a 1024 canvas the rounded square is 824 across, centred,
// which is a 0.0977 margin and a corner about 0.225 of the tile.

// The tile every variant sits on, written out in each rather than shared
// through a macro: a macro cannot expand to comma-separated array elements
// here, and two explicit lines read better than the workaround would.
//   Op::Fill { c: CLEAR },
//   Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },

// ------------------------------------------------------------ shipped mark

// A filled mouse showing only the wheel, on a rimmed black tile.
//
// Filled rather than outlined because a stroke is the first thing downscaling
// destroys, and this has to survive 16 pixels in a Finder list. Only the wheel,
// because drawing the button split as well put a second mark directly above it,
// and below about 64 pixels the gap between the two closes and they read as one
// long slot rather than as a mouse. Rimmed because a flat black tile has no
// edge whatsoever on a dark desktop.
//
// The tile follows Apple's own icon silhouette rather than approximating it:
// measured against six system icons, the shape is 80.5% of the canvas with a
// 0.0977 margin, and a superellipse exponent of 5.5 tracks their outline to
// within 1.3 pixels RMS at 256 across.
#[rustfmt::skip]
const ICON: &[Op] = RIM;

// ------------------------------------------------------------ candidates
//
// Kept so the choice is reviewable and reversible. Render one with
// `icon <dir> <name>`; nothing but ICON is ever built into the app.

const VARIANTS: &[(&str, &[Op])] = &[
    ("outline", OUTLINE),
    ("solid", SOLID),
    ("solid-ticks", SOLID_TICKS),
    ("pulse", PULSE),
    ("mouse-scale", MOUSE_SCALE),
    ("caliper", CALIPER),
    ("wheel", WHEEL),
    ("wheel-notch", WHEEL_NOTCH),
    ("scale2", SCALE2),
    ("pulse5", PULSE5),
    ("mouse-pulse", MOUSE_PULSE),
    ("invert", INVERT),
    ("rim", RIM),
];

/// The shipped mark with a grey rim around the tile. A flat black tile has no
/// edge at all on a dark desktop, and the app's own interface draws a grey rule
/// around every group, so the rim is the same device rather than decoration.
#[rustfmt::skip]
const RIM: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: 0.42 },
    Op::Squircle { x: 0.1097, y: 0.1097, w: 0.7807, h: 0.7807, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.315, y: 0.230, w: 0.370, h: 0.570, c: WHITE },
    Op::Capsule { x: 0.457, y: 0.330, w: 0.086, h: 0.180, c: BLACK },
];

/// Filled mouse showing ONLY the wheel. Drawing the button split as well is
/// what turned the earlier attempts into a single long slot: at any size below
/// about 64 pixels the gap between split and wheel closes and the two read as
/// one cut. One mark cannot close up against anything.
#[rustfmt::skip]
const WHEEL: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.315, y: 0.230, w: 0.370, h: 0.570, c: WHITE },
    Op::Capsule { x: 0.457, y: 0.330, w: 0.086, h: 0.180, c: BLACK },
];

/// The same, with the split shown as a notch cut into the top edge instead of
/// a line running down the body, so it cannot merge with the wheel.
#[rustfmt::skip]
const WHEEL_NOTCH: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.315, y: 0.230, w: 0.370, h: 0.570, c: WHITE },
    Op::Rect { x: 0.470, y: 0.215, w: 0.060, h: 0.075, c: BLACK },
    Op::Capsule { x: 0.457, y: 0.345, w: 0.086, h: 0.175, c: BLACK },
];

/// Mouse beside a scale, with the graduations cut to three and thickened so
/// they still separate at 32 pixels.
#[rustfmt::skip]
const SCALE2: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.175, y: 0.245, w: 0.310, h: 0.530, c: WHITE },
    Op::Capsule { x: 0.294, y: 0.335, w: 0.072, h: 0.150, c: BLACK },
    Op::Rect { x: 0.580, y: 0.245, w: 0.072, h: 0.530, c: WHITE },
    Op::Rect { x: 0.652, y: 0.288, w: 0.170, h: 0.070, c: WHITE },
    Op::Rect { x: 0.652, y: 0.475, w: 0.104, h: 0.070, c: WHITE },
    Op::Rect { x: 0.652, y: 0.662, w: 0.170, h: 0.070, c: WHITE },
];

/// Interval train, five slots wide with the fourth missing.
#[rustfmt::skip]
const PULSE5: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Rect { x: 0.200, y: 0.310, w: 0.080, h: 0.380, c: WHITE },
    Op::Rect { x: 0.340, y: 0.310, w: 0.080, h: 0.380, c: WHITE },
    Op::Rect { x: 0.480, y: 0.310, w: 0.080, h: 0.380, c: WHITE },
    Op::Rect { x: 0.760, y: 0.310, w: 0.080, h: 0.380, c: WHITE },
];

/// Mouse over an interval train: the silhouette carries the small sizes, the
/// train says what is being done to it at the large ones.
#[rustfmt::skip]
const MOUSE_PULSE: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.345, y: 0.180, w: 0.310, h: 0.470, c: WHITE },
    Op::Capsule { x: 0.464, y: 0.262, w: 0.072, h: 0.150, c: BLACK },
    Op::Rect { x: 0.215, y: 0.700, w: 0.072, h: 0.150, c: WHITE },
    Op::Rect { x: 0.359, y: 0.700, w: 0.072, h: 0.150, c: WHITE },
    Op::Rect { x: 0.503, y: 0.700, w: 0.072, h: 0.150, c: WHITE },
    Op::Rect { x: 0.647, y: 0.700, w: 0.072, h: 0.150, c: WHITE },
];

/// Inverted: black mark on a white tile. Brighter in a Dock and unmissable on a
/// dark desktop, at the cost of being the one bright thing in an app that is
/// otherwise entirely black.
#[rustfmt::skip]
const INVERT: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: WHITE },
    Op::Capsule { x: 0.315, y: 0.230, w: 0.370, h: 0.570, c: BLACK },
    Op::Capsule { x: 0.457, y: 0.330, w: 0.086, h: 0.180, c: WHITE },
];

/// Outlined mouse. The stroke reads as a technical drawing rather than a
/// pictogram, which suits an instrument, but thin strokes are what small sizes
/// destroy first.
#[rustfmt::skip]
const OUTLINE: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.320, y: 0.235, w: 0.360, h: 0.560, c: WHITE },
    Op::Capsule { x: 0.382, y: 0.297, w: 0.236, h: 0.436, c: BLACK },
    Op::Line { x1: 0.500, y1: 0.250, x2: 0.500, y2: 0.400, w: 0.048, c: WHITE },
    Op::Capsule { x: 0.462, y: 0.430, w: 0.076, h: 0.130, c: WHITE },
];

/// Filled mouse. A solid mass survives downscaling far better than a stroke,
/// so this is the safe answer for the sizes that actually get looked at.
#[rustfmt::skip]
const SOLID: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.320, y: 0.235, w: 0.360, h: 0.560, c: WHITE },
    Op::Line { x1: 0.500, y1: 0.235, x2: 0.500, y2: 0.395, w: 0.050, c: BLACK },
    Op::Capsule { x: 0.462, y: 0.425, w: 0.076, h: 0.135, c: BLACK },
];

/// Filled mouse with a graduation scale cut into the body. The measurement
/// idea sits inside the silhouette rather than beside it, so the outline is
/// still the only thing that has to survive 16 pixels.
#[rustfmt::skip]
const SOLID_TICKS: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.320, y: 0.235, w: 0.360, h: 0.560, c: WHITE },
    Op::Line { x1: 0.500, y1: 0.235, x2: 0.500, y2: 0.380, w: 0.050, c: BLACK },
    Op::Rect { x: 0.372, y: 0.470, w: 0.256, h: 0.038, c: BLACK },
    Op::Rect { x: 0.372, y: 0.556, w: 0.170, h: 0.038, c: BLACK },
    Op::Rect { x: 0.372, y: 0.642, w: 0.256, h: 0.038, c: BLACK },
];

/// No mouse at all: an interval train with one report missing. This is what the
/// program actually measures, and it is the most distinctive shape here, but it
/// needs the app's name beside it to say what it is about.
#[rustfmt::skip]
const PULSE: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Rect { x: 0.215, y: 0.330, w: 0.072, h: 0.340, c: WHITE },
    Op::Rect { x: 0.359, y: 0.330, w: 0.072, h: 0.340, c: WHITE },
    Op::Rect { x: 0.503, y: 0.330, w: 0.072, h: 0.340, c: WHITE },
    // The gap where a fourth bar would be is the dropped report.
    Op::Rect { x: 0.791, y: 0.330, w: 0.072, h: 0.340, c: WHITE },
];

/// Mouse beside a graduated scale. Fills the square tile, which a lone mouse
/// silhouette cannot, and states the measurement idea explicitly.
#[rustfmt::skip]
const MOUSE_SCALE: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Capsule { x: 0.180, y: 0.250, w: 0.300, h: 0.520, c: WHITE },
    Op::Line { x1: 0.330, y1: 0.250, x2: 0.330, y2: 0.385, w: 0.046, c: BLACK },
    Op::Capsule { x: 0.296, y: 0.415, w: 0.068, h: 0.120, c: BLACK },
    Op::Rect { x: 0.560, y: 0.250, w: 0.062, h: 0.520, c: WHITE },
    Op::Rect { x: 0.622, y: 0.286, w: 0.150, h: 0.052, c: WHITE },
    Op::Rect { x: 0.622, y: 0.416, w: 0.092, h: 0.052, c: WHITE },
    Op::Rect { x: 0.622, y: 0.546, w: 0.150, h: 0.052, c: WHITE },
    Op::Rect { x: 0.622, y: 0.676, w: 0.092, h: 0.052, c: WHITE },
];

/// Mouse held between caliper jaws: measurement as an action rather than as a
/// readout.
#[rustfmt::skip]
const CALIPER: &[Op] = &[
    Op::Fill { c: CLEAR },
    Op::Squircle { x: 0.0977, y: 0.0977, w: 0.8047, h: 0.8047, n: 5.5, c: BLACK },
    Op::Rect { x: 0.170, y: 0.215, w: 0.660, h: 0.060, c: WHITE },
    Op::Rect { x: 0.170, y: 0.215, w: 0.060, h: 0.180, c: WHITE },
    Op::Rect { x: 0.770, y: 0.215, w: 0.060, h: 0.180, c: WHITE },
    Op::Capsule { x: 0.360, y: 0.380, w: 0.280, h: 0.430, c: WHITE },
    Op::Line { x1: 0.500, y1: 0.380, x2: 0.500, y2: 0.500, w: 0.046, c: BLACK },
    Op::Capsule { x: 0.468, y: 0.530, w: 0.064, h: 0.110, c: BLACK },
];
