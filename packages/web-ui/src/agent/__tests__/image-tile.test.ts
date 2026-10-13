import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'
import type { EffectScope } from 'vue'

// An image in a sent message is a tile, as in the agent's message
// (`file-previews.md` § Files named in messages): a 2560 × 1720 screenshot in
// a message 760 pixels wide shows 248 × 167, however the user's message came
// to name it. A page of happy-dom, which lays nothing out, so the block's
// width is given; about 10 ms.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since Vue asks for its document.
const { effectScope, shallowRef } = await import('vue')
const { useImageTile } = await import('../message-editor/image-tile')

let scope: EffectScope | null = null
afterEach(() => {
  scope?.stop()
  scope = null
  document.body.innerHTML = ''
})

/** A message image in a block of `width` pixels, whose bytes have a size once they arrive. */
function image(width: number, enabled = true) {
  const block = document.createElement('span')
  block.className = 'media-lone'
  Object.defineProperty(block, 'clientWidth', { value: width })
  const element = document.createElement('img')
  element.alt = 'The login page'
  block.append(element)
  document.body.append(block)
  const arrive = (size: { width: number; height: number }) => {
    Object.defineProperty(element, 'naturalWidth', { value: size.width, configurable: true })
    Object.defineProperty(element, 'naturalHeight', { value: size.height, configurable: true })
    Object.defineProperty(element, 'complete', { value: true, configurable: true })
  }
  scope = effectScope()
  const tile = scope.run(() => useImageTile(shallowRef(element), enabled))!
  return { element, arrive, tile }
}

test('a message’s image takes its tile when its bytes arrive, and the hover preview knows its size', () => {
  const { element, arrive, tile } = image(760)
  // Before its bytes, it is held within the tile's frame.
  tile.fit()
  expect([element.style.maxWidth, element.style.maxHeight]).toEqual(['248px', '240px'])
  arrive({ width: 2560, height: 1720 })
  tile.fit()
  expect([element.style.width, element.style.height]).toEqual(['248px', '167px'])
  expect(tile.picture.value).toMatchObject({ kind: 'image', size: { width: 2560, height: 1720 }, caption: 'The login page' })
})

test('a tall page stays within the tile’s height', () => {
  const { element, arrive, tile } = image(760)
  arrive({ width: 1000, height: 8000 })
  tile.fit()
  expect([element.style.width, element.style.height]).toEqual(['30px', '240px'])
})

test('the composer keeps its image as it is', () => {
  const { element, arrive, tile } = image(760, false)
  arrive({ width: 2560, height: 1720 })
  tile.fit()
  expect(element.getAttribute('style')).toBeNull()
  expect(tile.picture.value).toBeNull()
})
