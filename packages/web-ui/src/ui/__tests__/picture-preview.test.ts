import { expect, test } from 'bun:test'
import { previewBox, previewPlacement } from '../picture-preview'

// The size of a picture in its hover card and where the card goes, on numbers
// alone: a few milliseconds.

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

// The tall page on a 900 px window: a 630 px picture, and the card's padding
// and caption add 34 px, a 664 px card.
const tall = { width: 180, height: 630 }
const chrome = 34

test('a card goes above its picture, or below it when only there it fits whole', () => {
  expect(previewPlacement(tall, chrome, { above: 700, below: 700 })).toEqual({ side: 'top', picture: tall })
  expect(previewPlacement(tall, chrome, { above: 300, below: 700 })).toEqual({ side: 'bottom', picture: tall })
})

test('a card with no room on either side shrinks into the larger one, never covering its picture', () => {
  // 565 px above, less the 8 px gap to the picture and the 8 px to the page's edge: a 549 px card.
  expect(previewPlacement(tall, chrome, { above: 565, below: 300 })).toEqual({ side: 'top', picture: { width: 147, height: 515 } })
  expect(previewPlacement(tall, chrome, { above: 300, below: 565 })).toEqual({ side: 'bottom', picture: { width: 147, height: 515 } })
})

test('a small picture keeps its own size and is never enlarged', () => {
  expect(previewBox({ width: 48, height: 48 }, 2, page)).toEqual({ width: 24, height: 24 })
  expect(previewBox({ width: 1, height: 1 }, 2, page)).toEqual({ width: 1, height: 1 })
})
