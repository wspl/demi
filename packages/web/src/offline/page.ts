/**
 * The page's side of its one service worker (`offline/service-worker.ts`):
 * it registers the worker, answers the direct channel's raw requests the
 * worker hands it, and tells the worker the build it runs, which the worker
 * then keeps for opening offline (`web-application.md` § Installing and
 * opening offline).
 */
import { answerRawRequests } from '../direct/raw-bridge'

/** Where the built app and the development server serve the worker, so its scope is the whole origin. */
export const SERVICE_WORKER_URL = '/service-worker.js'

/** Where a build's hashed files lie. */
const HASHED = '/assets/'

/**
 * Registers the worker and serves it for the page's lifetime. A browser
 * without service workers, or a page served where it cannot register one,
 * fetches every byte from the relay and does not open offline.
 */
export function startServiceWorker(): void {
  const container = typeof navigator === 'undefined' ? undefined : navigator.serviceWorker
  if (!container)
    return
  container.register(SERVICE_WORKER_URL, { scope: '/', type: 'module' }).catch((error: unknown) => {
    // The page works without it; the reason is for a developer.
    console.warn('The service worker did not register', error)
  })
  answerRawRequests(container)
  // Vite's development server serves sources, which name no build: nothing
  // is kept for opening offline there.
  const build = import.meta.env.DEMI_WEB_BUILD
  if (!build)
    return
  // Once the page has loaded, every file it names is fetched and timed.
  const loaded = document.readyState === 'complete'
    ? Promise.resolve()
    : new Promise<void>((resolve) => window.addEventListener('load', () => resolve(), { once: true }))
  const announce = (worker: ServiceWorker | null | undefined) => {
    worker?.postMessage({ type: 'demi-page', build, files: fetchedFiles() })
  }
  void Promise.all([container.ready, loaded]).then(([registration]) => {
    announce(registration.active)
    // A newer worker that takes this page over, as on the first visit after
    // an upgrade, when the one ready named was the old one, hears it too.
    // It listens for the page's lifetime.
    container.addEventListener('controllerchange', () => announce(container.controller))
  })
}

/**
 * The build's hashed files this page fetched so far: those its document
 * names and those the web browser timed since, such as a part loaded on
 * demand. The worker keeps those it did not see fetched, as on the first
 * visit, which no worker answered yet.
 */
function fetchedFiles(): string[] {
  const named = [
    ...Array.from(document.querySelectorAll<HTMLScriptElement>('script[src]'), (script) => script.src),
    ...Array.from(document.querySelectorAll<HTMLLinkElement>('link[href]'), (link) => link.href),
    ...performance.getEntriesByType('resource').map((entry) => entry.name),
  ]
  const files = new Set<string>()
  for (const address of named) {
    const url = new URL(address, location.href)
    if (url.origin === location.origin && url.pathname.startsWith(HASHED))
      files.add(url.pathname)
  }
  return [...files]
}
