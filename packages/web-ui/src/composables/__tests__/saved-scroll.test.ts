import { afterEach, expect, test } from 'bun:test'
import { savedScrollPosition } from '../useSavedScroll'

const realStorage = Reflect.get(globalThis, 'sessionStorage')

function storing(text: string | null): void {
  Reflect.set(globalThis, 'sessionStorage', { getItem: () => text })
}

afterEach(() => {
  Reflect.set(globalThis, 'sessionStorage', realStorage)
})

test('a complete snapshot is restored', () => {
  storing('{"top":120,"anchor":"block-3","offset":-8}')

  expect(savedScrollPosition('chat')).toEqual({
    top: 120,
    anchor: 'block-3',
    offset: -8,
  })
})

test('a snapshot that does not match is ignored whole', () => {
  storing('{"top":"1","anchor":null,"offset":0}')
  expect(savedScrollPosition('chat')).toBeNull()

  storing('{"top":10,"offset":0}')
  expect(savedScrollPosition('chat')).toBeNull()

  storing('{"top":-1,"anchor":null,"offset":0}')
  expect(savedScrollPosition('chat')).toBeNull()

  storing('not json')
  expect(savedScrollPosition('chat')).toBeNull()

  storing(null)
  expect(savedScrollPosition('chat')).toBeNull()
})

test('storage that cannot be read leaves the viewport at the top', () => {
  Reflect.set(globalThis, 'sessionStorage', {
    getItem() {
      throw new Error('Storage is disabled')
    },
  })

  expect(savedScrollPosition('chat')).toBeNull()
})
