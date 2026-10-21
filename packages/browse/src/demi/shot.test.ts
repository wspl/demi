// `demi.shot` of the slot's Chromium page and of a `demi.webkit()` page
// beside it, each started in a temporary slot: about 3 s, the two browsers'
// starts and a transition of 300 ms. No cheaper test shows which engine's
// picture a shot holds, or when the page's transitions let it be taken.
import { afterAll, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium, webkit, type Page } from 'playwright'
import { Browser } from '../browser'
import { Network } from '../network'
import { pngSize } from '../shots'
import { slotPorts, type Slot } from '../slot'
import type { Tool } from '../tool'
import { shot } from './shot'
import { webkitPage } from './webkit'

const root = mkdtempSync(join(tmpdir(), 'browse-shot-'))
const slot: Slot = { root, number: 0, ports: slotPorts(0), folder: join(root, '.cache/browse') }
mkdirSync(slot.folder, { recursive: true })
const browser = new Browser(slot, () => undefined)
const releases: (() => Promise<void>)[] = []
// The pages here are set by hand: the network stands in front of nothing.
const network = Network.listen(0, 1)
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
afterAll(async () => {
  for (const release of releases) {
    await release()
  }
  await browser.close()
  await (await network).shutdown()
  rmSync(root, { recursive: true, force: true })
})

/** The colour of the top left pixel of the PNG at `path`, as `page`'s engine decodes it. */
function firstPixel(page: Page, path: string): Promise<number[]> {
  return page.evaluate(async (bytes) => {
    const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }))
    const canvas = new OffscreenCanvas(1, 1)
    canvas.getContext('2d')!.drawImage(bitmap, 0, 0)
    return [...canvas.getContext('2d')!.getImageData(0, 0, 1, 1).data.slice(0, 3)]
  }, [...readFileSync(path)])
}

test.skipIf(!existsSync(chromium.executablePath()))(
  'a shot right after a click shows where a transition the click starts a frame later ends',
  async () => {
    const slotPage = await browser.page()
    // As Vue's Transition does, the drawer gets its final class two frames after the click.
    await slotPage.setContent(`<body style="margin: 0">
<div id="drawer" style="position: fixed; inset: 0; background: #f00; transition: background-color 300ms linear"></div>
<button style="position: fixed; right: 0; bottom: 0" onclick="requestAnimationFrame(() => requestAnimationFrame(() => { drawer.style.backgroundColor = '#00f' }))">Open</button>
</body>`)
    await slotPage.getByRole('button', { name: 'Open' }).click()
    const path = await shot(tool, join(root, 'drawer'))
    expect(await firstPixel(slotPage, path)).toEqual([0, 0, 255])
  },
)

test.skipIf(!existsSync(chromium.executablePath()))(
  'a shot of an element taller than the viewport, in a pane that scrolls, fails and says why',
  async () => {
    const slotPage = await browser.page()
    // As the gallery lays out: a fixed header over a pane whose content scrolls.
    await slotPage.setContent(`<body style="margin: 0; height: 100vh; overflow: hidden">
<header style="position: fixed; top: 0; height: 40px; width: 100%; background: #00f"></header>
<main style="height: 100%; overflow: auto"><div id="tall" style="height: 5000px; background: #f00"></div></main>
</body>`)
    await expect(shot(tool, join(root, 'tall'), { element: slotPage.locator('#tall') }))
      .rejects.toThrow('past the viewport')
  },
)

test.skipIf(!existsSync(chromium.executablePath()) || !existsSync(webkit.executablePath()))(
  'a shot of a WebKit page shows that page at the slot\'s pixel ratio, whether named by page or by element',
  async () => {
    const slotPage = await browser.page()
    await slotPage.setContent('<body style="margin: 0; background: #f00"></body>')
    const safari = await webkitPage(tool)
    await safari.setContent('<body style="margin: 0; background: #00f"><p id="note" style="margin: 0; width: 10px; height: 10px"></p></body>')
    const region = await shot(tool, join(root, 'region'), { page: safari, region: { x: 0, y: 0, width: 10, height: 10 } })
    const element = await shot(tool, join(root, 'element'), { element: safari.locator('#note') })
    expect([pngSize(region), await firstPixel(safari, region), pngSize(element), await firstPixel(safari, element)])
      .toEqual(['20×20 px', [0, 0, 255], '20×20 px', [0, 0, 255]])
  },
)
