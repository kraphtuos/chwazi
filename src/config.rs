//! Tunable constants and the colour palette.

// ---------------------------------------------------------------------------
// Tunables
// ---------------------------------------------------------------------------

/// Seconds the set of fingers must stay unchanged before a pick fires. The
/// countdown arc sweeps a full circle over exactly this time.
pub(crate) const SELECT_DELAY: f64 = 2.0;
/// Duration of the "pick one" flood flooding in around the winner.
pub(crate) const ANIM_ONE: f64 = 0.6;
/// Duration of the order/teams reveal animation.
pub(crate) const ANIM_OTHER: f64 = 1.4;
/// Solid centre disc radius as a fraction of the ring radius (leaves a gap).
pub(crate) const INNER_FRAC: f64 = 0.68;
/// Ring stroke width as a fraction of the base radius.
pub(crate) const RING_W: f64 = 0.16;
/// Radius of the clear "spotlight" window kept around the winning ring while
/// the rest of the screen floods, as a multiple of the base radius.
pub(crate) const HOLE_FRAC: f64 = 1.85;
/// Rotation speed (rad/s) of the shaded wedge as it grows around each ring.
pub(crate) const SPIN_SPEED: f64 = 4.5;
/// Buffer after each add before the countdown may start. During it the newly
/// added ring gradually grows in from its disc, so a still-settling touch never
/// triggers an instant pick.
pub(crate) const RING_BUILD: f64 = 0.5;
/// Beat the flooded winner is held after release, before the flood exits.
pub(crate) const HOLD_TIME: f64 = 0.8;
/// Duration of the flood receding and the winner dissolving away on release.
pub(crate) const EXIT_ONE: f64 = 0.7;
/// Duration of the quick flood whoosh-out when a result is cancelled by touch.
pub(crate) const EXIT_FAST: f64 = 0.32;
/// Time for a lifted dot to shrink out of the canvas. Driven off an absolute
/// timestamp (not per-frame integration) so it stays smooth even when frames
/// are dropped during the busy lift transition.
pub(crate) const SHRINK_TIME: f64 = 0.38;
/// Dead zone for a held virtual dot, as a fraction of the base radius. A
/// finger resting on a dot jitters a few px; only a move beyond this counts as
/// a deliberate drag and restarts the countdown, so a still press behaves like
/// a held finger and lets the pick fire.
pub(crate) const DRAG_SLOP: f64 = 0.3;

/// A single bob drives each finger's disc and ring together, forever. Fingers
/// are desynced from one another by a per-id phase. (amplitude, rad·s⁻¹)
pub(crate) const BOB_AMP: f64 = 0.075;
pub(crate) const BOB_FREQ: f64 = 4.0;

pub(crate) type Rgb = (u8, u8, u8);

/// Distinct, vivid colours assigned to fingers as they touch down. Spread
/// around the hue wheel so any two picked at random stay easy to tell apart.
pub(crate) const FINGER_COLORS: &[Rgb] = &[
    (255, 59, 48),  // red
    (255, 130, 0),  // orange
    (255, 204, 0),  // yellow
    (180, 210, 0),  // chartreuse
    (52, 199, 89),  // green
    (0, 190, 140),  // teal
    (0, 200, 210),  // cyan
    (90, 170, 255), // sky blue
    (0, 110, 255),  // blue
    (90, 80, 230),  // indigo
    (150, 90, 240), // violet
    (200, 70, 230), // purple
    (255, 60, 180), // magenta
    (255, 60, 110), // pink
    (170, 120, 90), // brown
];

/// Colours used to tint each team in "groups" mode.
pub(crate) const GROUP_COLORS: &[Rgb] = &[
    (255, 69, 58),
    (10, 132, 255),
    (48, 209, 88),
    (255, 214, 10),
    (191, 90, 242),
    (255, 159, 10),
    (50, 215, 190),
    (255, 55, 95),
];
