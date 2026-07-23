// Suppress iOS's double-tap gestures over the app (zoom, and the
// text-selection / "look up" loupe that pops up as a glassy peek on
// iOS 26). `touch-action: none` handles most of it; these cover the rest.
document.addEventListener('gesturestart', (e) => e.preventDefault());
document.addEventListener('gesturechange', (e) => e.preventDefault());
// A synthesized double-click is what triggers the glass peek — kill it.
document.addEventListener('dblclick', (e) => e.preventDefault(), { passive: false });

// iOS commits to the double-tap gesture on the SECOND touchstart, so
// preventing it only on touchend is too late — that is why the peek
// survived. Guard tightly so ordinary play is untouched: a second
// single-finger tap landing in nearly the same spot within 350ms. That
// is the double-tap gesture and not something a player does on purpose —
// two real fingers land apart, not within 40px of each other.
//
// Safe for the app's own input: WebKit fires pointerdown before
// touchstart, so the finger is already registered by the time this runs;
// preventing the touchstart default only suppresses the system gesture.
const TAP_MS = 350, TAP_PX = 40;
let lastTap = 0, lastX = 0, lastY = 0;
const nearLast = (t) =>
    Math.abs(t.clientX - lastX) < TAP_PX && Math.abs(t.clientY - lastY) < TAP_PX;

document.addEventListener('touchstart', (e) => {
    if (e.target.closest && e.target.closest('#ui, #dots')) return;
    if (e.touches.length !== 1) return;
    const t = e.changedTouches[0];
    if (t && Date.now() - lastTap <= TAP_MS && nearLast(t)) e.preventDefault();
}, { capture: true, passive: false });

document.addEventListener('touchend', (e) => {
    if (e.target.closest && e.target.closest('#ui, #dots')) return;
    const t = e.changedTouches[0];
    const now = Date.now();
    if (t && now - lastTap <= TAP_MS && nearLast(t)) e.preventDefault();
    lastTap = now;
    if (t) { lastX = t.clientX; lastY = t.clientY; }
}, { capture: true, passive: false });
