import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, jest, mock, test } from 'bun:test'

// A file icon's box has its size from the start and keeps it, and the same
// element, as the list of icons and the glyph's picture arrive; it holds nothing
// for 300 ms at most while they are on their way, then a plain outline. One Vue
// app on a happy-dom page, fake timers; a few ms.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick, shallowRef } = await import('vue')
const { REGION_WAIT_MS, regionsChanged, resetRegionWaits } = await import('../../infra/region-wait')

const { shallowReactive } = await import('vue')
const theme = shallowRef<unknown>(null)
/** The pictures that have arrived, by glyph. */
const pictures = shallowReactive(new Map<string, string | null>())
void mock.module('@demicodes/web-ui/files/file-icon-theme', () => ({
  fileIconTheme: theme,
  ensureFileIconTheme: async () => {},
  iconPicture: (glyph: string) => pictures.get(glyph),
}))
const { default: FileIcon } = await import('../FileIcon.vue')

const manifest = {
  file: 'file',
  folder: 'folder',
  folderNames: {},
  fileNames: {},
  fileExtensions: { ts: 'typescript' },
  iconDefinitions: {
    file: { iconPath: './../icons/file.svg' },
    folder: { iconPath: './../icons/folder.svg' },
    typescript: { iconPath: './../icons/typescript.svg' },
  },
}

let unmount: (() => void) | null = null
beforeEach(() => {
  jest.useFakeTimers()
  resetRegionWaits()
  theme.value = null
  pictures.clear()
})
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
  jest.useRealTimers()
})

function mountIcon(): HTMLElement {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(FileIcon, { name: 'app.ts', isDirectory: false, size: 20 }) })
  app.mount(root)
  unmount = () => app.unmount()
  return root
}

test('the box keeps its size and its element as the list and the picture arrive within 300 ms, and no outline shows', async () => {
  const root = mountIcon()
  const box = root.firstElementChild as HTMLElement
  const size = () => [box.style.width, box.style.height]
  expect(size()).toEqual(['20px', '20px'])
  expect(box.children).toHaveLength(0)

  jest.advanceTimersByTime(REGION_WAIT_MS - 100)
  theme.value = manifest
  regionsChanged()
  await nextTick()
  expect(root.firstElementChild).toBe(box)
  expect(box.children).toHaveLength(0)

  jest.advanceTimersByTime(50)
  pictures.set('icons/typescript.svg', '/icons/typescript.svg')
  regionsChanged()
  await nextTick()
  expect(root.firstElementChild).toBe(box)
  expect(size()).toEqual(['20px', '20px'])
  expect(box.querySelector('img')?.getAttribute('src')).toBe('/icons/typescript.svg')
  expect(box.querySelector('svg')).toBeNull()
})

test('past 300 ms the box holds a plain outline of the same size, which stays when the list arrives and the picture replaces in the same box', async () => {
  const root = mountIcon()
  const box = root.firstElementChild as HTMLElement
  jest.advanceTimersByTime(REGION_WAIT_MS)
  await nextTick()
  expect(box.querySelector('svg')).not.toBeNull()
  expect(box.querySelector('img')).toBeNull()
  expect([box.style.width, box.style.height]).toEqual(['20px', '20px'])

  // The list arrives, the picture does not: the outline stays, and no new wait starts.
  theme.value = manifest
  regionsChanged()
  await nextTick()
  expect(box.querySelector('svg')).not.toBeNull()
  jest.advanceTimersByTime(REGION_WAIT_MS * 2)
  await nextTick()
  expect(box.querySelector('svg')).not.toBeNull()

  pictures.set('icons/typescript.svg', '/icons/typescript.svg')
  regionsChanged()
  await nextTick()
  expect(root.firstElementChild).toBe(box)
  expect(box.querySelector('img')).not.toBeNull()
  expect(box.querySelector('svg')).toBeNull()
})
