// Keep the full-screen canvas covering the visible viewport, sized from JS.
//
// Why not pure CSS? On an installed iOS PWA a fixed canvas set to 100% /
// 100vh / 100dvh does NOT reliably fill the screen on a COLD LAUNCH: the first
// layout resolves against a viewport that iOS then "settles" to its real size a
// beat later, and it fires no event when it does. Measuring the viewport and
// writing the canvas size in px gets that first launch right.
//
// Why a rAF loop instead of events? Because that settle fires no event, an
// event-driven fit — or a fixed timer schedule — can miss it and lock the
// canvas at a too-small size (the old bug: a strip left uncovered until the app
// was quit and relaunched). Re-measuring every frame catches the settle with no
// event dependency, and there is no persisted "best" size to get stuck on.
//
// Why apply only once the size stops moving? A rotation sweeps the viewport
// through many intermediate sizes (and momentarily swapped width/height).
// Resizing the canvas to every one of them churns/flashes before it lands, so
// we wait until the measurement has held steady for a few frames and then snap
// once to the final size. Shrinking is allowed (portrait after landscape is
// narrower), so this must NOT be a never-shrink cache.
(function () {
    const c = document.getElementById('c');
    const STEADY = 2; // frames a new size must hold before it's applied
    let appliedW = 0, appliedH = 0; // size currently on the canvas
    let pendW = -1, pendH = -1, held = 0; // latest measurement + how long it's held

    function fit() {
        const vv = window.visualViewport;
        const w = Math.ceil(vv ? vv.width : window.innerWidth);
        const h = Math.ceil(vv ? vv.height : window.innerHeight);

        // Track how many consecutive frames this measurement has stayed put.
        if (w === pendW && h === pendH) {
            held++;
        } else {
            pendW = w; pendH = h; held = 0;
        }

        // Apply a change once it has settled — or immediately on the very first
        // frame, so a cold launch fills without waiting.
        if ((w !== appliedW || h !== appliedH) && (held >= STEADY || appliedW === 0)) {
            appliedW = w; appliedH = h;
            c.style.width = w + 'px';
            c.style.height = h + 'px';
        }

        // A reload can restore a stray scroll offset that nudges the fixed
        // canvas off the top edge on iOS.
        if (window.scrollX || window.scrollY) window.scrollTo(0, 0);
        requestAnimationFrame(fit);
    }
    requestAnimationFrame(fit);
})();
