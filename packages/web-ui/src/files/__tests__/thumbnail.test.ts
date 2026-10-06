import { expect, test } from 'bun:test'
import { thumbnailBox } from '../thumbnail'

// The box a tool's image and an image in a run of a message's images take
// (`file-previews.md` § Media a tool returned): 80 pixels tall, 64 to 200
// wide by the image's proportions, cropped beyond them, never enlarged, and
// the narrowest box while the image's size is unknown. Pure arithmetic;
// it costs nothing.

const cases: { name: string; natural: { width: number; height: number } | null; box: { width: number; height: number } }[] = [
  { name: 'a screenshot keeps its proportions', natural: { width: 1280, height: 720 }, box: { width: 142, height: 80 } },
  { name: 'a whole page is cropped to the narrowest box', natural: { width: 360, height: 2400 }, box: { width: 64, height: 80 } },
  { name: 'a wide strip is cropped to the widest box', natural: { width: 2000, height: 200 }, box: { width: 200, height: 80 } },
  { name: 'a small icon keeps its own size', natural: { width: 50, height: 50 }, box: { width: 50, height: 50 } },
  { name: 'a short strip is cropped at its own scale', natural: { width: 300, height: 40 }, box: { width: 200, height: 40 } },
  { name: 'an image not yet loaded takes the narrowest box', natural: null, box: { width: 64, height: 80 } },
]

for (const { name, natural, box } of cases) {
  test(name, () => {
    expect(thumbnailBox(natural)).toEqual(box)
  })
}
