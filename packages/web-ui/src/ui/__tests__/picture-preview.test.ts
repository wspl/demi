import { expect, test } from 'bun:test'
import { previewBox } from '../picture-preview'

// The size of a picture in its hover card (`file-previews.md` § A picture's
// preview on hover, The card), on numbers alone: a few milliseconds.

const page = { width: 1440, height: 900 }

test('a picture shows at its own size, its pixels one to one on the screen', () => {
  expect(previewBox({ width: 2000, height: 295 }, 2, page)).toEqual({ width: 1000, height: 148 })
  expect(previewBox({ width: 2000, height: 295 }, 1, { width: 3000, height: 2000 })).toEqual({ width: 2000, height: 295 })
})

test('a picture larger than 70% of the page is scaled down to fit it', () => {
  expect(previewBox({ width: 2560, height: 1600 }, 2, page)).toEqual({ width: 1008, height: 630 })
  expect(previewBox({ width: 2560, height: 1600 }, 2, { width: 2000, height: 1200 })).toEqual({ width: 1280, height: 800 })
  expect(previewBox({ width: 360, height: 2400 }, 2, page)).toEqual({ width: 95, height: 630 })
})

test('a small picture keeps its own size and is never enlarged', () => {
  expect(previewBox({ width: 48, height: 48 }, 2, page)).toEqual({ width: 24, height: 24 })
  expect(previewBox({ width: 1, height: 1 }, 2, page)).toEqual({ width: 1, height: 1 })
})
