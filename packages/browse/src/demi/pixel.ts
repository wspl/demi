// `demi.pixel(x, y)`: the colour the page shows at a point, in CSS pixels of
// the viewport, as its screenshot has it.
import type { Tool } from '../tool'

export interface Colour {
  red: number
  green: number
  blue: number
  /** As `#rrggbb`. */
  hex: string
}

export async function pixel(tool: Tool, x: number, y: number): Promise<Colour> {
  const page = await tool.browser.page()
  const picture = await tool.browser.capture({ clip: { x, y, width: 1, height: 1 } })
  // The browser decodes its own picture: the page's canvas reads the first
  // pixel without touching the page's document.
  const [red, green, blue] = await page.evaluate(async (bytes) => {
    const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }))
    const canvas = new OffscreenCanvas(1, 1)
    const drawing = canvas.getContext('2d')
    if (!drawing) {
      throw new Error('no 2D canvas')
    }
    drawing.drawImage(bitmap, 0, 0)
    return [...drawing.getImageData(0, 0, 1, 1).data]
  }, [...picture])
  const hex = `#${[red, green, blue].map((channel) => channel.toString(16).padStart(2, '0')).join('')}`
  return { red, green, blue, hex }
}
