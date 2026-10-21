import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { compile } from '@tailwindcss/node'
import { chromium, type Browser, type Page } from 'playwright'
import { vueSingleFileComponents, withoutViteGlobs } from '../../__tests__/vue-components'

// The transcript's list knows a long user message, folded under Show More, by
// its folded height, also once the message has left the list again. Several
// long messages the list draws at once, as the reader scrolls to them or as
// it opens at them, where each one's unfolded height would push the next out
// of the list before it folds, leave no empty space in the list, which is as
// tall however and however often it was reached. Show Less at the end of an
// opened message leaves the message in view, its Show More where the Show
// Less was.
//
// Heights are layout, so this runs the product's UserBlock in the list's
// virtualizer in a real Chromium with the stylesheet Tailwind builds from it;
// happy-dom lays nothing out. About 4 s: the page's and the stylesheet's
// builds, the browser's start, and five lists each settling over frames.

/** The list as the page measures it. */
interface ListState {
  /** The list's whole height. */
  total: number
  /** The heights the list knows the messages by, in order. */
  known: (number | undefined)[]
  /** The first message's row as drawn; null while the list does not draw it. */
  drawn: number | null
}

declare global {
  interface Window {
    /**
     * Mounts a list 600 px tall of six long messages, each with its answer,
     * and thirty answers after them. It opens at its end, or where the reader
     * left it, at its top.
     */
    showList(at: 'end' | 'top'): void
    /** Scrolls the list to its top or its end, as the reader does. */
    scrollList(to: 'top' | 'end'): void
    listState(): ListState
  }
}

const PAGE_ENTRY = `
import { createApp, defineComponent, h, ref } from 'vue'
import UserBlock from '../blocks/UserBlock.vue'
import { provideBlobUrl } from '../media-source'
import { useBlockVirtualizer } from '../../composables/useBlockVirtualizer'

const pasted = Array.from({ length: 300 }, (_, index) => 'Line ' + (index + 1) + ' of the pasted build log: the step finished and wrote its output.').join('\\n\\n')
let list
let scroller

window.showList = (at) => {
  const blocks = ref([])
  const root = document.createElement('div')
  document.body.append(root)
  createApp(defineComponent({
    setup() {
      provideBlobUrl(() => '')
      const container = ref()
      const left = { anchor: { blockId: 'message-0', anchorIndex: 0, offsetPx: 0, scrollTop: 0 }, heightCache: new Map() }
      list = useBlockVirtualizer(container, blocks, at === 'top' ? left : undefined)
      return () => h('div', {
        ref: (el) => { container.value = el ?? undefined; scroller = container.value },
        // The list places its rows itself, as the transcript's does.
        style: 'width:700px;height:600px;overflow-y:auto;overflow-anchor:none',
        onScroll: list.onScroll,
      }, [
        h('div', { style: { position: 'relative', height: list.totalSize.value + 'px' } }, list.virtualItems.value.map((item) => h('div', {
          key: String(item.key),
          'data-index': item.index,
          'data-row-key': String(item.key),
          ref: (el) => list.measureElement(el),
          style: { position: 'absolute', top: 0, left: 0, right: 0, transform: 'translateY(' + item.start + 'px)' },
        }, [
          blocks.value[item.index].type === 'user'
            ? h(UserBlock, { content: [{ type: 'text', text: pasted }] })
            : h('p', { style: 'height:100px;margin:0' }, 'Answer ' + item.index),
        ]))),
      ])
    },
  })).mount(root)
  // The rows arrive after the list, as a conversation's history does.
  blocks.value = [
    ...Array.from({ length: 6 }, (_, index) => [{ id: 'message-' + index, type: 'user' }, { id: 'reply-' + index, type: 'text' }]).flat(),
    ...Array.from({ length: 30 }, (_, index) => ({ id: 'answer-' + index, type: 'text' })),
  ]
}

window.scrollList = (to) => {
  scroller.scrollTop = to === 'top' ? 0 : scroller.scrollHeight
  scroller.dispatchEvent(new Event('scroll'))
}

window.listState = () => {
  const row = document.querySelector('[data-row-key="message-0"]')
  return {
    total: list.totalSize.value,
    known: Array.from({ length: 6 }, (_, index) => list.virtualizer.value.itemSizeCache.get('message-' + index)),
    drawn: row ? row.getBoundingClientRect().height : null,
  }
}
`

describe.skipIf(!existsSync(chromium.executablePath()))('in Chromium', () => {
  let browser: Browser
  let script: string
  let css: string
  beforeAll(async () => {
    const entry = fileURLToPath(new URL('./folded-message-height-entry.ts', import.meta.url))
    const build = await Bun.build({
      entrypoints: [entry],
      files: { [entry]: PAGE_ENTRY },
      plugins: [vueSingleFileComponents, withoutViteGlobs],
      target: 'browser',
      conditions: ['development'],
      define: { 'process.env.NODE_ENV': '"development"' },
    })
    if (!build.success)
      throw new AggregateError(build.logs, 'The page did not build')
    script = await build.outputs[0]!.text()
    const styles = fileURLToPath(new URL('../../styles/', import.meta.url))
    const compiler = await compile(`@import 'tailwindcss';\n@import './base.css';`, { base: styles, onDependency: () => {} })
    css = compiler.build([...new Set(script.split(/[\s'"`\\]+/))])
    browser = await chromium.launch()
  })
  afterAll(async () => {
    await browser?.close()
  })

  /**
   * The list once nothing in it moves: its height, its scroll position and
   * the rows it draws the same for ten frames in a row.
   */
  function settled(opened: Page): Promise<ListState> {
    return opened.evaluate(() => new Promise<ListState>((resolve) => {
      let last = ''
      let same = 0
      const frame = () => {
        const state = window.listState()
        const now = JSON.stringify(state) + document.querySelector('[data-row-key]')?.parentElement?.parentElement?.scrollTop
        same = now === last ? same + 1 : 0
        last = now
        if (same >= 10)
          resolve(state)
        else
          requestAnimationFrame(frame)
      }
      requestAnimationFrame(frame)
    }))
  }

  async function load(at: 'end' | 'top'): Promise<{ opened: Page, state: ListState }> {
    const opened = await browser.newPage({ viewport: { width: 1000, height: 800 }, reducedMotion: 'reduce' })
    await opened.route('https://demi.test/', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }))
    await opened.goto('https://demi.test/')
    await opened.addStyleTag({ content: css })
    await opened.addScriptTag({ content: script, type: 'module' })
    await opened.waitForFunction(() => typeof window.showList === 'function')
    await opened.evaluate((place) => window.showList(place), at)
    return { opened, state: await settled(opened) }
  }

  /** The list once the reader scrolled to `to` and it settled there. */
  async function scrolled(opened: Page, to: 'top' | 'end'): Promise<ListState> {
    await opened.evaluate((place) => window.scrollList(place), to)
    return settled(opened)
  }

  test('long messages drawn at once are known by their folded height, and the list is as tall each time', async () => {
    const { opened } = await load('end')
    const first = await scrolled(opened, 'top')
    // The first message shows folded, under Show More, and the list knows
    // every message by that height, those it no longer draws too.
    expect(first.drawn).toBeLessThan(400)
    expect(await opened.getByRole('button', { name: 'Show More' }).count()).toBeGreaterThan(0)
    expect(first.known).toEqual(Array.from({ length: 6 }, () => first.drawn!))

    // To the end and back, the list is as tall as it was.
    await scrolled(opened, 'end')
    expect((await scrolled(opened, 'top')).total).toBe(first.total)

    // Loaded again and read from the top, it is as tall again.
    const again = await load('end')
    expect((await scrolled(again.opened, 'top')).total).toBe(first.total)
    await again.opened.close()

    // Opened where the reader left it, its messages drawn as it mounts, too.
    const reopened = await load('top')
    expect(reopened.state.known).toEqual(first.known)
    expect(reopened.state.total).toBe(first.total)
    await reopened.opened.close()
    await opened.close()
  })

  test('Show Less at the end of an opened message leaves its Show More where the Show Less was', async () => {
    const { opened } = await load('end')
    await scrolled(opened, 'top')
    // A message far enough down that the list has room above it to keep it in
    // place.
    const message = opened.locator('[data-row-key="message-2"]')
    await message.getByRole('button', { name: 'Show More' }).click()
    await settled(opened)
    // The reader reads down to the message's end.
    const less = message.getByRole('button', { name: 'Show Less' })
    await less.scrollIntoViewIfNeeded()
    await settled(opened)
    const at = (await less.boundingBox())!.y
    await less.click()
    await settled(opened)
    // The message is still drawn, rather than the list showing its end.
    const fold = message.getByRole('button', { name: 'Show More' })
    expect(await fold.count()).toBe(1)
    const more = (await fold.boundingBox())!.y
    expect(Math.abs(more - at)).toBeLessThan(2)
    await opened.close()
  })
})
