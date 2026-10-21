// A timeline whose action awaits a transition: about 2 s for Chromium and
// a 300 ms animation. Only its painted frames show which moment it kept.
import { afterAll, expect, test } from 'bun:test'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'
import { Browser } from '../browser'
import { Network } from '../network'
import { slotPorts, type Slot } from '../slot'
import type { Tool } from '../tool'
import { timeline } from './timeline'

const root = mkdtempSync(join(tmpdir(), 'browse-timeline-'))
const slot: Slot = { root, number: 0, ports: slotPorts(0), folder: join(root, '.cache/browse') }
mkdirSync(slot.folder, { recursive: true })
const browser = new Browser(slot, () => undefined)
const network = Network.listen(0, 1)
const tool: Tool = {
  slot,
  browser,
  network: () => network,
  print: () => undefined,
  env: {},
  wrote: () => undefined,
  release: () => undefined,
  endServer: () => undefined,
}
afterAll(async () => {
  await browser.close()
  await (await network).shutdown()
  rmSync(root, { recursive: true, force: true })
})

test.skipIf(!existsSync(chromium.executablePath()))('a timeline keeps moments from the action starting, not from its awaited animation ending', async () => {
  const page = await browser.page()
  await page.setContent('<body style="margin: 0; background: red"></body>')
  const paths = await timeline(tool, () => page.evaluate(async () => {
    document.body.style.background = '#0f0'
    await document.body.animate([{ opacity: 1 }, { opacity: 1 }], { duration: 300 }).finished
    document.body.style.background = '#00f'
  }), [100, 500], { region: { x: 0, y: 0, width: 1, height: 1 } })
  const colors = []
  for (const path of paths) {
    colors.push(await page.evaluate(async (bytes) => {
      const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }))
      const canvas = new OffscreenCanvas(1, 1)
      const context = canvas.getContext('2d')!
      context.drawImage(bitmap, 0, 0)
      return [...context.getImageData(0, 0, 1, 1).data.slice(0, 3)]
    }, [...readFileSync(path)]))
  }
  expect(colors).toEqual([[0, 255, 0], [0, 0, 255]])
})
