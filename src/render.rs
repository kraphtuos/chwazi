//! All canvas drawing: the flood, each finger's disc + ring, and overlays.

use std::f64::consts::TAU;

use crate::config::*;
use crate::state::{App, Mode, Phase};
use crate::util::{lerp_col, rgba, smoothstep};

/// Everything `draw_ring` needs to render one finger's disc + ring. Bundled so
/// the call sites read by name instead of a long positional argument list.
struct RingSpec {
    x: f64,
    y: f64,
    /// Resting ring radius (px); the bob and `scale` are applied inside.
    r: f64,
    col: Rgb,
    alpha: f64,
    scale: f64,
    /// Build-in progress 0→1: the ring unfurls from `mid` to a full circle.
    ring_in: f64,
    /// Per-ring seed that desyncs the bob (usually the pointer id).
    seed: f64,
    /// Angle the ring unfurls from and the bob is referenced to.
    mid: f64,
    /// `Some((start_angle, countdown_fraction))` draws the light track plus the
    /// growing wedge; `None` draws a solid settled ring.
    arc: Option<(f64, f64)>,
}

impl App {
    pub(crate) fn draw(&self) {
        let ctx = &self.ctx;
        let _ = ctx.set_transform(self.dpr, 0.0, 0.0, self.dpr, 0.0, 0.0);
        ctx.clear_rect(0.0, 0.0, self.width, self.height);

        // Pick One, once decided: the winner's colour floods the screen but
        // leaves a clear round "spotlight" so the winning ring stays visible.
        // This holds after the fingers lift, then fades out.
        if self.mode == Mode::One
            && matches!(
                self.phase,
                Phase::Animate { .. } | Phase::Result | Phase::Hold { .. }
            )
        {
            if let (Some(col), Some((wx, wy))) = (self.flood_col, self.flood_pos) {
                self.draw_flood(wx, wy, col);
                return;
            }
        }

        if matches!(self.phase, Phase::Idle) && self.fingers.is_empty() {
            self.draw_hint();
        }

        // While gathering, each ring starts fully in the lighter colour and the
        // shaded (saturated) area grows to a full circle as the countdown runs —
        // but only once at least two fingers are down. The instant it completes,
        // the pick fires. `Some(fraction)` means "draw the lighter track"; the
        // fraction is 0 until there are ≥2 fingers.
        let count = match self.phase {
            Phase::Gather { changed } => {
                // The wedge stays empty through the ring build-in buffer, then
                // sweeps to full over `SELECT_DELAY`.
                let cd = if self.active_count() >= 2 && self.real_alive() >= 1 {
                    ((self.now - changed - RING_BUILD) / SELECT_DELAY).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                Some(cd)
            }
            _ => None,
        };

        for id in self.fingers.keys().copied() {
            self.draw_finger(id, count);
        }

        // A cancelled result whooshes away on top of the fresh round.
        if let Some((t0, col, (cx, cy))) = self.cancel {
            self.draw_cancel(t0, col, cx, cy);
        }
    }

    /// A cancelled result's flood, whooshing back out to the edges. Full-opacity
    /// fill receding purely by the growing hole — identical to the Hold-timeout
    /// exit in `draw_flood`, so a touch that hands the flood off mid-exit is
    /// seamless (no brightness pop) as well as continuous in hole size.
    fn draw_cancel(&self, t0: f64, col: Rgb, cx: f64, cy: f64) {
        let e = ((self.now - t0) / EXIT_FAST).clamp(0.0, 1.0);
        let cover = self.width.hypot(self.height);
        let spot_r = self.base_r() * 1.25 * HOLE_FRAC;
        let hole = spot_r + (cover - spot_r) * smoothstep(e);
        if hole >= cover {
            return;
        }
        self.fill_with_hole(col, cx, cy, hole);
    }

    /// Fill the whole canvas with `col`, punching a clear circular hole of
    /// radius `hole` at (`hx`, `hy`): the rect is wound one way and the hole the
    /// other, so the nonzero-winding rule cuts the hole out. A hole ≤1px is
    /// dropped (a plain full-screen fill). Shared by the Pick One flood and the
    /// cancel overlay, which use identical geometry.
    fn fill_with_hole(&self, col: Rgb, hx: f64, hy: f64, hole: f64) {
        let ctx = &self.ctx;
        ctx.save();
        ctx.set_fill_style_str(&rgba(col, 1.0));
        ctx.begin_path();
        ctx.rect(0.0, 0.0, self.width, self.height);
        if hole > 1.0 {
            ctx.move_to(hx + hole, hy);
            let _ = ctx.arc_with_anticlockwise(hx, hy, hole, 0.0, TAU, true);
        }
        ctx.fill();
        ctx.restore();
    }

    /// Draw one finger's disc + gap + ring. `count` is the countdown fraction
    /// (`Some` while gathering) that grows the shaded wedge toward a full circle.
    fn draw_finger(&self, id: i32, count: Option<f64>) {
        let f = &self.fingers[&id];
        if f.alpha * f.scale <= 0.01 {
            return;
        }
        let base = self.base_r();
        let col = if self.mode == Mode::Groups && self.reveal > 0.0 {
            lerp_col(
                f.color,
                GROUP_COLORS[f.group % GROUP_COLORS.len()],
                self.reveal,
            )
        } else {
            f.color
        };

        // How far the ring has grown in from its disc since this finger landed.
        let ring_in = smoothstep(((self.now - f.birth) / RING_BUILD).clamp(0.0, 1.0));

        // (wedge start angle, countdown fraction). The start angle spins, so the
        // growing shaded wedge also rotates. Desynced per finger.
        //
        // The angle MUST be reduced modulo a full turn before it reaches
        // ctx.arc(): iOS reports enormous pointer ids (~1.7e9), so `id * 2.3`
        // is an astronomically large angle that Safari's canvas fails to draw —
        // the wedge then never renders, grows, or spins. `rem_euclid(TAU)` keeps
        // it small while preserving both the spin and the per-finger offset.
        // A finger lifted *before* a pick keeps the normal light-track ring —
        // just without the countdown wedge and a touch faded — as it shrinks
        // out. Forcing the light track (`Some` with a zero wedge) matters once
        // the last finger lifts and the phase flips to Idle: then `count` is
        // None, which would otherwise draw the solid *saturated* ring, making
        // the ring suddenly match the central disc's colour on the way out.
        // Once a result is on screen the ring instead keeps its result look all
        // the way out, so the ranked/team rings don't go pale as they leave.
        let (arc, alpha) = if f.depart.is_some() && self.reveal <= 0.0 {
            (Some((0.0, 0.0)), f.alpha * 0.7)
        } else {
            (
                count.map(|c| ((self.spin + id as f64 * 2.3).rem_euclid(TAU), c)),
                f.alpha,
            )
        };
        self.draw_ring(RingSpec {
            x: f.x,
            y: f.y,
            r: f.ring,
            col,
            alpha,
            scale: f.scale,
            ring_in,
            seed: id as f64,
            mid: f.birth_angle,
            arc,
        });

        // Rank number for order mode, centred in the disc and big enough to read
        // at a glance. It sits under the player's own fingertip while they press,
        // which is fine — the post-release hold is when everyone reads it.
        // Teams needs no number: the team colour already says it.
        if self.reveal > 0.0 && self.mode == Mode::Order && f.rank > 0 {
            let sz = (base * 0.9 * f.scale) as i32;
            let ctx = &self.ctx;
            ctx.save();
            ctx.set_global_alpha(f.alpha * self.reveal * f.scale);
            ctx.set_text_align("center");
            ctx.set_text_baseline("middle");
            ctx.set_fill_style_str("rgba(255,255,255,0.98)");
            ctx.set_font(&format!("700 {sz}px system-ui, sans-serif"));
            let _ = ctx.fill_text(&f.rank.to_string(), f.x, f.y);
            ctx.restore();
        }
    }

    /// Solid centre disc, a gap, then the ring. `arc` is `(start_angle,
    /// countdown_fraction)`: the ring starts fully in the lighter tint and a
    /// shaded (saturated) wedge grows from the start angle to a full circle as
    /// the countdown runs, the start angle spinning so the wedge also rotates.
    fn draw_ring(&self, spec: RingSpec) {
        let RingSpec {
            x,
            y,
            r,
            col,
            alpha,
            scale,
            ring_in,
            seed,
            mid,
            arc,
        } = spec;
        if alpha * scale <= 0.01 {
            return;
        }
        let ctx = &self.ctx;
        let base = self.base_r();

        // One bob drives the disc and the ring together, seeded per pointer so
        // fingers stay out of step with each other. (The seed is wrapped so
        // iOS's ~1.7e9 ids keep the sine argument well-conditioned.) The
        // amplitude scales with the object's own `scale`, so the bob ramps in as
        // it grows and — crucially — damps to nothing as it shrinks out, keeping
        // the exit a smooth, monotonic shrink instead of a wobble that reads as
        // stutter.
        let bob =
            1.0 + BOB_AMP * scale * (self.now * BOB_FREQ + (seed * 2.3).rem_euclid(TAU)).sin();

        // Floors at 0 (not a positive minimum) so a shrinking object glides all
        // the way to nothing instead of stalling at a few pixels — the object is
        // culled by the alpha·scale check above once it's effectively gone.
        let r = (r * scale).max(0.0);
        let lw = ((base * RING_W).max(4.0) * scale).max(0.0);
        let disc_r = (r * INNER_FRAC * bob).max(0.0);
        let ring_r = r * bob;

        // The ring draws itself on from a single point (`mid`), extending both
        // ways until it closes into a full circle (`ring_in`: 0 → 1).
        let span = ring_in * TAU;
        let (a0, a1) = (mid - span / 2.0, mid + span / 2.0);

        ctx.save();
        ctx.set_global_alpha(alpha * scale);

        // Solid filled centre disc (always the saturated colour).
        ctx.set_fill_style_str(&rgba(col, 1.0));
        ctx.begin_path();
        let _ = ctx.arc(x, y, disc_r, 0.0, TAU);
        ctx.fill();

        // Round caps only while the ring is a partial arc (building in). A full
        // circle uses butt caps so the two ends meet cleanly instead of stacking
        // a round cap that reads as a bump — especially as the ring shrinks out.
        let cap = if ring_in < 0.999 { "round" } else { "butt" };
        match arc {
            Some((start, cd)) => {
                // Lighter "track" — sweeps on from the point as the ring builds.
                let light = lerp_col(col, (255, 255, 255), 0.55);
                ctx.set_line_cap(cap);
                ctx.set_line_width(lw);
                ctx.set_stroke_style_str(&rgba(light, 1.0));
                ctx.begin_path();
                let _ = ctx.arc(x, y, ring_r, a0, a1);
                ctx.stroke();

                // The shaded (saturated) wedge grows 0 → full over the countdown
                // while its whole span rotates. Rounded ends look best here.
                if cd > 0.0015 {
                    let len = TAU * cd.min(1.0);
                    ctx.set_line_cap("round");
                    ctx.set_line_width(lw);
                    ctx.set_stroke_style_str(&rgba(col, 1.0));
                    ctx.begin_path();
                    let _ = ctx.arc(x, y, ring_r, start, start + len);
                    ctx.stroke();
                }
                ctx.set_line_cap("butt");
            }
            None => {
                // Solid saturated ring (settled) — also draws on both ways while
                // building in.
                ctx.set_line_cap(cap);
                ctx.set_line_width(lw);
                ctx.set_stroke_style_str(&rgba(col, 1.0));
                ctx.begin_path();
                let _ = ctx.arc(x, y, ring_r, a0, a1);
                ctx.stroke();
                ctx.set_line_cap("butt");
            }
        }

        ctx.restore();
    }

    /// The Pick One spotlight-flood. The winner's colour *zooms* out from the
    /// finger to fill the screen (accelerating), then a clear round spotlight
    /// irises open around the winning ring. Holds, then fades, after lift.
    fn draw_flood(&self, wx: f64, wy: f64, col: Rgb) {
        let base = self.base_r();
        let cover = self.width.hypot(self.height);
        let seed = self.winner.unwrap_or(0) as f64;
        let wr = base * (1.0 + 0.25 * self.reveal.clamp(0.0, 1.0));
        let spot_r = wr * HOLE_FRAC;

        // The whole thing is one clear "spotlight" circle punched out of a
        // full-screen colour fill. As it shrinks from covering everything down
        // to just the ring, the colour appears to flood IN from the edges. On
        // release it opens back out to the edges and the colour recedes. A
        // single smooth interpolation for both, so neither stutters.
        let (hole, ws) = match self.phase {
            Phase::Hold { start } => {
                let t = self.now - start;
                if t < HOLD_TIME {
                    // Hold the full flood: winner sits visible and bobbing.
                    (spot_r, 1.0)
                } else {
                    // Flood and winner recede together, one smooth curve. The
                    // bob is damped by `scale` (see draw_ring), so this shrink
                    // stays monotonic.
                    let e = ((t - HOLD_TIME) / EXIT_ONE).clamp(0.0, 1.0);
                    let k = smoothstep(e);
                    (spot_r + (cover - spot_r) * k, 1.0 - k)
                }
            }
            _ => {
                // Flood in from the edges; ease-out so it starts sooner.
                let r = self.reveal.clamp(0.0, 1.0);
                let fp = 1.0 - (1.0 - r) * (1.0 - r);
                (cover + (spot_r - cover) * fp, 1.0)
            }
        };

        // Losing rings sit under the incoming colour and get swallowed edge-in.
        if !matches!(self.phase, Phase::Hold { .. }) {
            for (id, f) in self.fingers.iter() {
                if Some(*id) != self.winner {
                    self.draw_ring(RingSpec {
                        x: f.x,
                        y: f.y,
                        r: f.ring,
                        col: f.color,
                        alpha: f.alpha,
                        scale: f.scale,
                        ring_in: 1.0,
                        seed: *id as f64,
                        mid: f.birth_angle,
                        arc: None,
                    });
                }
            }
        }

        // Full-screen colour with the clear spotlight punched out.
        self.fill_with_hole(col, wx, wy, hole);

        // The winning ring sits ON TOP, in the clear spotlight, still bobbing.
        if ws > 0.01 {
            self.draw_ring(RingSpec {
                x: wx,
                y: wy,
                r: wr,
                col,
                alpha: 1.0,
                scale: ws,
                ring_in: 1.0,
                seed,
                mid: -std::f64::consts::FRAC_PI_2,
                arc: None,
            });
        }
    }

    fn draw_hint(&self) {
        let ctx = &self.ctx;
        let (cx, cy) = (self.width / 2.0, self.height / 2.0);
        ctx.save();
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        ctx.set_fill_style_str("rgba(255,255,255,0.85)");
        ctx.set_font("600 26px system-ui, sans-serif");
        let _ = ctx.fill_text("Everyone, place a finger", cx, cy - 14.0);
        ctx.set_fill_style_str("rgba(255,255,255,0.5)");
        ctx.set_font("400 17px system-ui, sans-serif");
        let _ = ctx.fill_text("Hold still \u{2014} it picks after a moment", cx, cy + 20.0);
        ctx.restore();
    }

    /// Small dev-only overlay showing when this build was produced (local time),
    /// so you can tell at a glance whether the device is running fresh WASM.
    #[cfg(debug_assertions)]
    pub(crate) fn draw_build_stamp(&self) {
        use wasm_bindgen::JsValue;

        let secs: f64 = env!("BUILD_UNIX").parse().unwrap_or(0.0);
        let d = js_sys::Date::new(&JsValue::from_f64(secs * 1000.0));
        let ih = crate::util::window()
            .inner_height()
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        // Where the canvas actually sits on screen, so the stamp can be pinned
        // to the visible viewport edges even though the canvas is overscanned.
        let rect = self.canvas.get_bounding_client_rect();
        let s = format!(
            "dev {:02}:{:02}:{:02}",
            d.get_hours(),
            d.get_minutes(),
            d.get_seconds(),
        );
        let ctx = &self.ctx;
        ctx.save();
        ctx.set_text_align("left");
        ctx.set_text_baseline("bottom");
        ctx.set_fill_style_str("rgba(255,255,255,0.6)");
        ctx.set_font("500 11px ui-monospace, Menlo, monospace");
        // Placed against the VISIBLE viewport edges (the canvas is overscanned,
        // so its own edges are off-screen). `-rect.*` converts viewport 0 into
        // canvas coordinates; extra offset clears the home indicator.
        let x = 10.0 - rect.left();
        let y = ih - rect.top() - 44.0;
        let _ = ctx.fill_text(&s, x, y);
        ctx.restore();
    }
}
