// Minimal offline cache. Optional: the app HTML is fully self-contained, so
// this only adds "works with no network on repeat visits / when installed".
//
// NETWORK-FIRST (was cache-first). Cache-first pinned the installed PWA to the
// exact build captured at install time: refreshes kept serving the old HTML and
// WASM forever, so app updates never appeared without a full reinstall. Now a
// refresh fetches fresh content whenever the network is reachable and only falls
// back to the cache when offline.
const CACHE = 'chwazi-v3';

self.addEventListener('install', (e) => {
    self.skipWaiting();
    e.waitUntil(
        caches.open(CACHE).then((c) => c.addAll(['./', './index.html']).catch(() => { }))
    );
});

self.addEventListener('activate', (e) => {
    e.waitUntil(
        caches.keys().then((keys) =>
            Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))
        ).then(() => self.clients.claim())
    );
});

self.addEventListener('fetch', (e) => {
    if (e.request.method !== 'GET') return;
    e.respondWith(
        fetch(e.request)
            .then((res) => {
                // Refresh the cache copy for offline use.
                const copy = res.clone();
                caches.open(CACHE).then((c) => c.put(e.request, copy).catch(() => { }));
                return res;
            })
            .catch(() =>
                // Offline: serve whatever we cached, falling back to the shell.
                caches.match(e.request).then((hit) => hit || caches.match('./index.html'))
            )
    );
});
