// `demi.host` against a stand-in of the header's Host menu, in Playwright's own Chromium, which
// `bun browse` installs on its first call. The stand-in speaks Japanese: the call finds the
// button by the hook `HostMenu` gives it and the device by its name, never by the page's words.
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { chromium, type Browser } from 'playwright'
import { Failure } from '../tool'
import { hostOn } from './host'

/**
 * The button names the Host the conversation runs on, and its menu lists the Cloud and the
 * devices, a name with its state under it. A device row marked `data-pinned` is one a
 * conversation with messages would ask about: the button keeps its name.
 */
const PAGE = `<!doctype html>
<button data-host-menu aria-label="実行場所" id="host">クラウド</button>
<div id="menu" role="menu" hidden>
  <div role="menuitem"><span>クラウド</span></div>
  <div role="menuitem" data-device="runner-a"><span>runner-a</span><span>オンライン</span></div>
  <div role="menuitem" data-device="runner-b" aria-disabled="true"><span>runner-b</span><span>オフライン</span></div>
  <div role="menuitem" data-device="runner-c" data-pinned><span>runner-c</span><span>オンライン</span></div>
</div>
<script>{
  const host = document.getElementById('host')
  const menu = document.getElementById('menu')
  host.addEventListener('click', () => { menu.hidden = !menu.hidden })
  onkeydown = (event) => { if (event.key === 'Escape') menu.hidden = true }
  for (const row of menu.querySelectorAll('[data-device]')) {
    row.addEventListener('click', () => {
      menu.hidden = true
      if (row.getAttribute('aria-disabled') === 'true' || row.hasAttribute('data-pinned')) return
      host.textContent = row.dataset.device
    })
  }
}</script>`

let browser: Browser | null = null

beforeAll(async () => {
  if (existsSync(chromium.executablePath())) {
    browser = await chromium.launch()
  }
})

afterAll(async () => {
  await browser?.close()
})

test.skipIf(!existsSync(chromium.executablePath()))(
  'the conversation runs on the device the call names, and a device that is missing, offline or refuses to move fails the call with the reason',
  async () => {
    const page = await browser!.newPage()
    await page.setContent(PAGE)
    await hostOn(page, 'runner-a')
    expect(await page.locator('#host').innerText()).toBe('runner-a')

    await page.setContent(PAGE)
    const missing = await hostOn(page, 'runner-z').catch((error) => error)
    expect(missing).toBeInstanceOf(Failure)
    expect(missing.message).toContain('runner-z')
    expect(missing.message).toContain('runner-a')

    const offline = await hostOn(page, 'runner-b').catch((error) => error)
    expect(offline).toBeInstanceOf(Failure)
    expect(offline.message).toContain('runner-b is not online')

    const asked = await hostOn(page, 'runner-c').catch((error) => error)
    expect(asked).toBeInstanceOf(Failure)
    expect(asked.message).toContain('asks before it moves')
    await page.close()
  },
  30_000,
)
