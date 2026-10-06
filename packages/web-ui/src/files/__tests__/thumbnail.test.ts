import { expect, test } from 'bun:test'
import type { ToolMediaSource } from '@demicodes/protocol'
import { declaredSize, thumbnailBox } from '../thumbnail'

// The box a tool's image or video and one in a run of a message's media take
// (`file-previews.md` § Media a tool returned): 80 pixels tall, 64 to 200
// wide by the medium's proportions, cropped beyond them, never enlarged, and
// while the size is unknown the narrowest box for an image and 16:9 for a
// video. Pure arithmetic; it costs nothing.

const cases: { name: string; natural: { width: number; height: number } | null; kind?: 'image' | 'video'; box: { width: number; height: number } }[] = [
  { name: 'a screenshot keeps its proportions', natural: { width: 1280, height: 720 }, box: { width: 142, height: 80 } },
  { name: 'a whole page is cropped to the narrowest box', natural: { width: 360, height: 2400 }, box: { width: 64, height: 80 } },
  { name: 'a wide strip is cropped to the widest box', natural: { width: 2000, height: 200 }, box: { width: 200, height: 80 } },
  { name: 'a small icon keeps its own size', natural: { width: 50, height: 50 }, box: { width: 50, height: 50 } },
  { name: 'a short strip is cropped at its own scale', natural: { width: 300, height: 40 }, box: { width: 200, height: 40 } },
  { name: 'an image of unknown size takes the narrowest box', natural: null, box: { width: 64, height: 80 } },
  { name: 'a portrait recording keeps its proportions', natural: { width: 720, height: 1280 }, kind: 'video', box: { width: 64, height: 80 } },
  { name: 'a video of unknown size takes a 16:9 box', natural: null, kind: 'video', box: { width: 142, height: 80 } },
]

for (const { name, natural, kind, box } of cases) {
  test(name, () => {
    expect(thumbnailBox(natural, kind)).toEqual(box)
  })
}

// A reference that carries its medium's size gives the final box before a
// byte of the medium loads; one without it leaves the size unknown.
test('a reference with its size gives the box before the bytes arrive', () => {
  const shot: ToolMediaSource = { type: 'ref', ref: 'a'.repeat(64), mediaType: 'image/png', width: 360, height: 2400 }
  expect(thumbnailBox(declaredSize(shot), 'image')).toEqual({ width: 64, height: 80 })
  const wide: ToolMediaSource = { ...shot, width: 2000, height: 200 }
  expect(thumbnailBox(declaredSize(wide), 'image')).toEqual({ width: 200, height: 80 })
  const clip: ToolMediaSource = { type: 'ref', ref: 'b'.repeat(64), mediaType: 'video/webm' }
  expect(declaredSize(clip)).toBeNull()
})
