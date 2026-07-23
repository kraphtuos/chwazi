//! Low-level helpers: DOM/window access, the rAF loop, event-listener glue,
//! easing curves, RNG, and colour formatting.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Document, HtmlElement, Window};

use crate::config::Rgb;
use crate::state::APP;

pub(crate) fn window() -> Window {
    web_sys::window().expect("no window")
}

pub(crate) fn document() -> Document {
    window().document().expect("no document")
}

pub(crate) fn with_app<F: FnOnce(&mut crate::state::App)>(f: F) {
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            f(app);
        }
    });
}

pub(crate) fn request_frame(cb: &Closure<dyn FnMut(f64)>) {
    window()
        .request_animation_frame(cb.as_ref().unchecked_ref())
        .expect("rAF failed");
}

pub(crate) fn start_raf() {
    // The standard wasm-bindgen self-referencing rAF loop.
    #[allow(clippy::type_complexity)]
    let f: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let g = f.clone();
    *g.borrow_mut() = Some(Closure::wrap(Box::new(move |ts: f64| {
        with_app(|app| app.frame(ts));
        request_frame(f.borrow().as_ref().unwrap());
    }) as Box<dyn FnMut(f64)>));
    request_frame(g.borrow().as_ref().unwrap());
}

pub(crate) fn add_listener<T, E, F>(target: &T, ev: &str, f: F)
where
    T: AsRef<web_sys::EventTarget>,
    E: wasm_bindgen::convert::FromWasmAbi + 'static,
    F: FnMut(E) + 'static,
{
    let cb = Closure::wrap(Box::new(f) as Box<dyn FnMut(E)>);
    target
        .as_ref()
        .add_event_listener_with_callback(ev, cb.as_ref().unchecked_ref())
        .expect("addEventListener failed");
    cb.forget();
}

/// Toggle a `hidden` class on an element without disturbing its other classes.
pub(crate) fn set_hidden(el: &HtmlElement, hidden: bool) {
    let cls = el.class_name();
    let has = cls.split_whitespace().any(|c| c == "hidden");
    if hidden && !has {
        el.set_class_name(&format!("{cls} hidden"));
    } else if !hidden && has {
        let n: Vec<&str> = cls.split_whitespace().filter(|c| *c != "hidden").collect();
        el.set_class_name(&n.join(" "));
    }
}

/// Classic Hermite ease (0→0, 1→1) with zero slope at both ends.
pub(crate) fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Inverse of `smoothstep`: given `y` in 0..1, the `t` such that
/// `smoothstep(t) == y`. Used to hand an animation off at its current progress
/// so a follow-on (e.g. a cancel) continues from where it is instead of jumping.
pub(crate) fn inv_smoothstep(y: f64) -> f64 {
    let y = y.clamp(0.0, 1.0);
    0.5 - (((1.0 - 2.0 * y).asin()) / 3.0).sin()
}

pub(crate) fn rnd() -> f64 {
    js_sys::Math::random()
}

pub(crate) fn rnd_idx(n: usize) -> usize {
    ((rnd() * n as f64) as usize).min(n.saturating_sub(1))
}

pub(crate) fn shuffle<T>(v: &mut [T]) {
    for i in (1..v.len()).rev() {
        v.swap(i, rnd_idx(i + 1));
    }
}

pub(crate) fn rgba(c: Rgb, a: f64) -> String {
    format!("rgba({},{},{},{:.3})", c.0, c.1, c.2, a.clamp(0.0, 1.0))
}

pub(crate) fn lerp_col(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let l = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t) as u8;
    (l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}
