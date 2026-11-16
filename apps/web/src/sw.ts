/// <reference lib="webworker" />
// Custom service worker (injectManifest): precaches the full build output so
// the app runs completely offline after the first load.

declare let self: ServiceWorkerGlobalScope & { __WB_MANIFEST: Array<{ url: string; revision: string | null }> };

// v2: entries are stored with their Vary header stripped (see storeClean).
// v1 caches contained `Vary: Origin` responses (added by the preview/static
// server), which made caches.match() miss for every page-originated asset
// request that carries an Origin header — the app then failed to boot offline.
const PRECACHE = "pigeon-precache-v2";
// Injected by vite-plugin-pwa (injectManifest).
const manifest = self.__WB_MANIFEST;

// The injected manifest can list the same URL twice (e.g. manifest.webmanifest);
// dedupe before caching.
const precacheUrls = [...new Set(manifest.map((entry) => entry.url))];

/** Returns a copy of the response without its Vary header. */
function stripVary(response: Response): Response {
  if (!response.headers.has("vary")) {
    return response;
  }
  const headers = new Headers(response.headers);
  headers.delete("vary");
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

/** Fetches one precache URL and stores it without Vary-sensitive metadata. */
async function storeClean(cache: Cache, url: string): Promise<void> {
  const response = await fetch(url);
  if (!response.ok && response.type !== "opaque") {
    throw new Error(`precache request for ${url} failed with ${response.status}`);
  }
  await cache.put(url, stripVary(response));
}

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(PRECACHE)
      .then((cache) => Promise.all(precacheUrls.map((url) => storeClean(cache, url))))
      .then(() => self.skipWaiting())
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((k) => k !== PRECACHE).map((k) => caches.delete(k)))
      )
      .then(async () => {
        // Prune entries from previous installs that are no longer part of the
        // app (e.g. hashed assets from an older build). Manifest URLs are
        // relative; cache keys are absolute, so normalize before comparing.
        const cache = await caches.open(PRECACHE);
        const current = new Set(precacheUrls.map((u) => new URL(u, self.location.origin).href));
        for (const key of await cache.keys()) {
          if (!current.has(key.url)) {
            await cache.delete(key);
          }
        }
      })
      .then(() => self.clients.claim())
  );
});

const matchOptions: CacheQueryOptions = {
  ignoreSearch: true,
  // Stored responses are Vary-free (v2), but keep matching robust against
  // caches written by older service workers.
  ignoreVary: true,
};

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") {
    return;
  }
  if (request.mode === "navigate") {
    event.respondWith(
      caches
        .match(request, matchOptions)
        .then((cached) => cached ?? caches.match("index.html", matchOptions))
        .then(
          (cached) =>
            cached ??
            fetch(request).then((response) => {
              const copy = response.clone();
              caches.open(PRECACHE).then((cache) => cache.put("index.html", stripVary(copy)));
              return response;
            })
        )
    );
    return;
  }
  event.respondWith(
    caches.match(request, matchOptions).then((cached) => cached ?? fetch(request))
  );
});

export {};
