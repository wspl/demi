// Where screenshots go: `.cache/browse/shots/<name>.png` in the slot, or the
// path a name gives when it has a folder in it, such as a report's folder
// outside the repository; and the parts of a picture a command keeps.
import { mkdirSync, readFileSync } from 'node:fs'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import type { Page } from 'playwright'
import { CommandFailure, numeric } from './command'
import { slotPaths, type Slot } from './slot'

export type Clip = { x: number, y: number, width: number, height: number }

export function shotPath(slot: Slot, name?: string): string {
  const chosen = name ?? `shot-${new Date().toISOString().slice(11, 23).replaceAll(':', '-')}`
  const file = chosen.endsWith('.png') ? chosen : `${chosen}.png`
  const path = file.includes('/')
    ? (isAbsolute(file) ? file : resolve(slot.root, file))
    : join(slotPaths(slot).shots, file)
  mkdirSync(dirname(path), { recursive: true })
  return path
}

/** The area `text` gives as x,y,width,height in CSS pixels, for the option `name`. */
export function region(text: string, name: string): Clip {
  const parts = text.split(',').map((part) => numeric(part, name))
  if (parts.length !== 4) {
    throw new CommandFailure(`${name} takes x,y,width,height, not ${text}`)
  }
  const [x, y, width, height] = parts
  return { x, y, width, height }
}

/** The pixel size the header of the PNG at `path` gives, as `2880×1800 px`. */
export function pngSize(path: string): string {
  const header = readFileSync(path).subarray(0, 24)
  return `${header.readUInt32BE(16)}×${header.readUInt32BE(20)} px`
}

/**
 * The part `clip` of the PNG `picture`, a picture of a viewport `viewportWidth`
 * CSS pixels wide, as a PNG. The browser decodes and encodes it: the
 * page's canvas does it without touching the page's document.
 */
export async function cropPng(page: Page, picture: Buffer, clip: Clip, viewportWidth: number): Promise<Buffer> {
  const bytes = await page.evaluate(async ({ png, area, width }) => {
    const whole = await createImageBitmap(new Blob([new Uint8Array(png)], { type: 'image/png' }))
    const ratio = whole.width / width
    const canvas = new OffscreenCanvas(Math.round(area.width * ratio), Math.round(area.height * ratio))
    const drawing = canvas.getContext('2d')
    if (!drawing) {
      throw new Error('no 2D canvas')
    }
    drawing.drawImage(whole, area.x * ratio, area.y * ratio, canvas.width, canvas.height, 0, 0, canvas.width, canvas.height)
    const blob = await canvas.convertToBlob({ type: 'image/png' })
    return [...new Uint8Array(await blob.arrayBuffer())]
  }, { png: [...picture], area: clip, width: viewportWidth })
  return Buffer.from(bytes)
}
