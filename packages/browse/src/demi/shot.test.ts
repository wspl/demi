// `demi.shot` of a `demi.webkit()` page beside the slot's Chromium page,
// each started in a temporary slot: about 2 s, the two browsers' starts. No
// cheaper test shows which engine's picture a shot holds.
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
