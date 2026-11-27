// SPDX-License-Identifier: GPL-3.0-only
// Keeps the player itself available offline and quick to start. Requests to music servers
// are never touched. The build replaces the version, so a new release replaces the cache.
const CACHE = 'infrasonic-__VERSION__';
const SHELL = [
  './',
  'index.html',
  'style.css',
  'config.js',
  'main.js',
  'worker.js',
  'audio-worklet.js',
  'manifest.webmanifest',
  'icon.svg',
  'pkg/infrasonic-web.js',
  'pkg/infrasonic-web_bg.wasm',
];
// The audio worker is cached when it first loads (on the first song), not at install, so
// that the first visit stays small.

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET' || url.origin !== self.location.origin) {
    return;
  }
  // The network first, so that deployments show up; the cache when offline.
  e.respondWith(
    fetch(e.request)
      .then((r) => {
        if (r.ok && url.pathname.startsWith(new URL('./', self.location).pathname)) {
          const copy = r.clone();
          caches.open(CACHE).then((c) => c.put(e.request, copy));
        }
        return r;
      })
      .catch(() => caches.match(e.request).then((r) => r || caches.match('index.html'))),
  );
});
