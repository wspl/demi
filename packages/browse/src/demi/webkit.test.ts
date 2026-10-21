// A page that checks its push subscription must remain usable in WebKit.
// About 2 s for two browsers and a local service worker; only the actual
// engine exposes a mismatch between its JavaScript API and its IPC endpoint.
import { afterAll, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium, webkit } from 'playwright'
import { Browser } from '../browser'
import { withinLimit } from '../limit'
import { Network } from '../network'
import { slotPorts, type Slot } from '../slot'
import type { Tool } from '../tool'
import { webkitPage } from './webkit'

const root = mkdtempSync(join(tmpdir(), 'browse-webkit-'))
const slot: Slot = { root, number: 0, ports: slotPorts(0), folder: join(root, '.cache/browse') }
mkdirSync(slot.folder, { recursive: true })
const browser = new Browser(slot, () => undefined)
const network = Network.listen(0, 1)
const releases: (() => Promise<void>)[] = []
const tool: Tool = {
  slot,
  browser,
  network: () => network,
  print: () => undefined,
  env: {},
  wrote: () => undefined,
  release: (release) => releases.push(release),
  endServer: () => undefined,
}
const server = Bun.serve({
  port: 0,
  hostname: '127.0.0.1',
  fetch(request) {
    if (new URL(request.url).pathname === '/worker.js') {
      return new Response('self.addEventListener("activate", event => event.waitUntil(self.clients.claim()))', {
        headers: { 'content-type': 'text/javascript' },
      })
    }
    return new Response(`<!doctype html><button onclick="this.textContent = 'Clicked'">Ready</button>
<script>
window.checked = navigator.serviceWorker.register('/worker.js')
  .then(() => navigator.serviceWorker.ready)
  .then(registration => {
    if (typeof PushManager !== 'undefined')
      return registration.pushManager.getSubscription()
  })
</script>`, { headers: { 'content-type': 'text/html' } })
  },
})

afterAll(async () => {
  for (const release of releases)
    await release()
  await browser.close()
  await (await network).shutdown()
  await server.stop(true)
  rmSync(root, { recursive: true, force: true })
})

test.skipIf(!existsSync(chromium.executablePath()) || !existsSync(webkit.executablePath()))(
  'a page can check push support and still answer evaluations and clicks after a reload',
  async () => {
    const page = await webkitPage(tool)
    const check = async () => {
      // The page checks only an API the browser advertises, as the product does.
      await page.evaluate('window.checked')
      await page.getByRole('button', { name: 'Ready' }).click()
      return page.getByRole('button', { name: 'Clicked' }).innerText()
    }
    await page.goto(server.url.href)
    expect(await withinLimit(check(), 4000)).toBe('Clicked')
    await page.reload()
    expect(await withinLimit(check(), 4000)).toBe('Clicked')
  },
  15_000,
)
