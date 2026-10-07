// `bun check pixel <x> <y>`: prints the colour the page shows at a point,
// in CSS pixels of the viewport, as its screenshot has it.
import { CheckFailure, numeric, parse, type Context } from '../command'

const USAGE = 'pixel <x> <y>'

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  if (positionals.length !== 2) {
    throw new CheckFailure(`Usage: bun check ${USAGE}`)
  }
  const x = numeric(positionals[0], 'x')
  const y = numeric(positionals[1], 'y')
  const page = await context.browser.page()
  const picture = await page.screenshot({ clip: { x, y, width: 1, height: 1 } })
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
  const hex = [red, green, blue].map((channel) => channel.toString(16).padStart(2, '0')).join('')
  context.print(`rgb(${red}, ${green}, ${blue})  #${hex}`)
}
