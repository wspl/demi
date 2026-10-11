import { expect, test } from 'bun:test'
import type { ToolMediaSource } from '@demicodes/protocol'
import { declaredSize, ROW_PICTURE, thumbnailBox, tileBox } from '../thumbnail'

// The tile a picture of a message takes, lone or in a run
// (`file-previews.md` § Files named in messages): it fits a third of the
// message's width less the two 8-pixel gaps, and 240 pixels tall, with its
// proportions kept and never enlarged. Pure arithmetic; it costs nothing.

const tiles: { name: string; natural: { width: number; height: number } | null; kind?: 'image' | 'video'; width: number; box: { width: number; height: number } | null }[] = [
  { name: 'a screenshot fits the width of a third of the message', natural: { width: 2560, height: 1720 }, width: 760, box: { width: 248, height: 167 } },
  { name: 'a whole page fits the height', natural: { width: 1000, height: 8000 }, width: 760, box: { width: 30, height: 240 } },
  { name: 'a small icon keeps its own size', natural: { width: 50, height: 50 }, width: 760, box: { width: 50, height: 50 } },
  { name: 'a narrower message makes a smaller tile', natural: { width: 2560, height: 1720 }, width: 376, box: { width: 120, height: 81 } },
  { name: 'a video of unknown size takes a 16:9 box as wide as the tile', natural: null, kind: 'video', width: 760, box: { width: 248, height: 140 } },
  { name: 'an image of unknown size has no box until it loads', natural: null, width: 760, box: null },
]

for (const { name, natural, kind, width, box } of tiles) {
  test(name, () => {
    expect(tileBox(natural, kind ?? 'image', width)).toEqual(box)
  })
}

test('three tiles fill a line with the two gaps between them', () => {
  const box = tileBox({ width: 4000, height: 1000 }, 'image', 760)!
  expect(3 * box.width + 2 * 8).toBe(760)
})

// A picture in a row's own line (`file-previews.md` § Media a tool returned):
// 20 pixels tall, 16 to 48 wide by its proportions, cropped beyond them,
// never enlarged, and while its size is unknown the narrowest box for an
// image and 16:9 for a video.

const rows: { name: string; natural: { width: number; height: number } | null; kind?: 'image' | 'video'; box: { width: number; height: number } }[] = [
  { name: 'a row picture keeps its proportions within its range', natural: { width: 1280, height: 720 }, box: { width: 36, height: 20 } },
  { name: 'a whole page in a row is cropped to the narrowest box', natural: { width: 360, height: 2400 }, box: { width: 16, height: 20 } },
  { name: 'a wide strip in a row is cropped to the widest box', natural: { width: 2000, height: 200 }, box: { width: 48, height: 20 } },
  { name: 'a small icon in a row keeps its own size', natural: { width: 16, height: 16 }, box: { width: 16, height: 16 } },
  { name: 'a row image of unknown size takes the narrowest box', natural: null, box: { width: 16, height: 20 } },
  { name: 'a row video of unknown size takes a 16:9 box', natural: null, kind: 'video', box: { width: 36, height: 20 } },
]

for (const { name, natural, kind, box } of rows) {
  test(name, () => {
    expect(thumbnailBox(natural, kind ?? 'image', ROW_PICTURE)).toEqual(box)
  })
}

// A reference that carries its medium's size gives the final box before a
// byte of the medium loads; one without it leaves the size unknown.
test('a reference with its size gives the box before the bytes arrive', () => {
  const shot: ToolMediaSource = { type: 'ref', ref: 'a'.repeat(64), mediaType: 'image/png', width: 2560, height: 1720 }
  expect(tileBox(declaredSize(shot), 'image', 760)).toEqual({ width: 248, height: 167 })
  const clip: ToolMediaSource = { type: 'ref', ref: 'b'.repeat(64), mediaType: 'video/webm' }
  expect(declaredSize(clip)).toBeNull()
})
