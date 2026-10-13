import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { chromium, type Browser, type Page } from 'playwright'
import type { useMarkdownBlocks as UseMarkdownBlocks } from '../useMarkdownBlocks'
import type { md as Md } from '../../markdown/md'
import type * as Vue from 'vue'

// A streaming reply renders a block at a time (`file-previews.md`): a
// settled block renders once and stays as the same nodes, the last block
// renders again as text arrives, and the render of the whole reply at its
// end replaces only the blocks it changes. In a real Chromium, for the
// message renderer's sanitizer and the fades' animations: about 1.5 s for
// the file, the build and the browser's start, shared by its tests.

/** What the page is given: Vue, the composable and the message renderer. */
interface Exposed {
  vue: typeof Vue
  useMarkdownBlocks: typeof UseMarkdownBlocks
  md: typeof Md
}

/** The page's handle on a reply rendering into `root`, for `page.evaluate`. */
declare global {
  interface Window {
    exposed: Exposed
    reply: {
      root: HTMLElement
      text: Vue.Ref<string>
      streaming: Vue.Ref<boolean>
      steps: Vue.ShallowRef<{ start: number; end: number; at: number }[]>
      /** The source of each render, in order. */
      renders: string[]
      /** Shows `text`, as the stream's next step does, and waits for the page to follow. */
      show(text: string, streaming?: boolean): Promise<void>
    }
  }
}

describe.skipIf(!existsSync(chromium.executablePath()))('in Chromium', () => {
  let browser: Browser
  let script: string
  beforeAll(async () => {
    const at = (path: string) => JSON.stringify(fileURLToPath(new URL(path, import.meta.url)))
    // A file of its own in the package, so its imports resolve from here.
    const entry = fileURLToPath(new URL('./page-entry.ts', import.meta.url))
    const build = await Bun.build({
      entrypoints: [entry],
      files: {
        [entry]: [
          `import * as vue from 'vue'`,
          `import { useMarkdownBlocks } from ${at('../useMarkdownBlocks.ts')}`,
          `import { md } from ${at('../../markdown/md.ts')}`,
          `Object.assign(window, { exposed: { vue, useMarkdownBlocks, md } })`,
        ].join('\n'),
      },
      target: 'browser',
      conditions: ['development'],
    })
    if (!build.success)
      throw new AggregateError(build.logs, 'The page did not build')
    script = await build.outputs[0]!.text()
    browser = await chromium.launch()
  })
  afterAll(async () => {
    await browser?.close()
  })

  /** A page with a reply rendering a block at a time, its text empty and streaming. */
  async function page(): Promise<Page> {
    const opened = await browser.newPage()
    // A page of an origin, as the renderer's modules read the page's storage.
    await opened.route('https://demi.test/', (route) => route.fulfill({ contentType: 'text/html', body: '<!doctype html><body></body>' }))
    await opened.goto('https://demi.test/')
    await opened.addScriptTag({ content: script })
    await opened.evaluate(() => {
      const { vue, useMarkdownBlocks, md } = window.exposed
      const root = document.createElement('div')
      document.body.append(root)
      const text = vue.ref('')
      const streaming = vue.ref(true)
      const steps = vue.shallowRef<{ start: number; end: number; at: number }[]>([])
      const renders: string[] = []
      vue.effectScope().run(() => useMarkdownBlocks({
        root: () => root,
        text: () => text.value,
        streaming: () => streaming.value,
        steps: () => steps.value,
        render: (source) => () => {
          renders.push(source())
          return md.render(source())
        },
        describe: () => null,
        rendered: () => {},
      }))
      window.reply = {
        root,
        text,
        streaming,
        steps,
        renders,
        async show(next, live = true) {
          text.value = next
          streaming.value = live
          await vue.nextTick()
        },
      }
    })
    return opened
  }

  /** An essay of `count` paragraphs, as a model writes one, each numbered to the same width. */
  function essay(count: number): string {
    return Array.from({ length: count }, (_, n) => `Paragraph ${String(n).padStart(4, '0')} weighs **the matter** at some length, as an essay does, and goes on.`).join('\n\n')
  }

  test('a settled block renders once and stays the same nodes, and the reply ends as its whole render', async () => {
    const reply = essay(12)
    const result = await (await page()).evaluate(async (reply) => {
      const { root, renders, show } = window.reply
      let first: Element | null = null
      for (let end = 22; end < reply.length + 22; end += 22) {
        await show(reply.slice(0, end))
        first ??= root.children.length > 1 ? root.children[0]! : null
      }
      const streamed = [...renders]
      const kept = root.contains(first)
      await show(reply, false)
      return { streamed, kept, ended: root.innerHTML, whole: window.exposed.md.render(reply) }
    }, reply)
    for (let n = 0; n < 10; n += 1) {
      const paragraph = `Paragraph ${String(n).padStart(4, '0')} weighs`
      // Its render as a block of its own, after which no render reads it.
      const settled = result.streamed.findIndex((source) => source.startsWith(paragraph) && source.indexOf('Paragraph', 1) < 0 && source.endsWith('\n\n'))
      expect({ n, settled: settled >= 0, after: result.streamed.slice(settled + 1).filter((source) => source.includes(paragraph)) }).toEqual({ n, settled: true, after: [] })
    }
    expect(result.kept).toBe(true)
    expect(result.ended).toBe(result.whole)
  })

  // A reference defined at the reply's end links text in its first block,
  // which only the whole reply decides.
  test('the render of the whole reply at its end replaces only the blocks it changes', async () => {
    const reply = 'See [the guide][guide] first.\n\nThe second paragraph stays.\n\nSo does the third.\n\n[guide]: https://example.com/guide\n'
    const result = await (await page()).evaluate(async (reply) => {
      const { root, show } = window.reply
      for (let end = 10; end < reply.length + 10; end += 10)
        await show(reply.slice(0, end))
      const before = [...root.children]
      await show(reply, false)
      const after = [...root.children]
      return {
        links: root.querySelectorAll('a[href="https://example.com/guide"]').length,
        kept: before.map((element) => after.includes(element)),
      }
    }, reply)
    expect(result).toEqual({ links: 1, kept: [false, true, true] })
  })

  // A step that starts in a paragraph and ends in the next fades on in the
  // first after it settles, which renders it then and never again, and the
  // fades never stack.
  test('a step that crosses into a new block fades on in the block that settles', async () => {
    const result = await (await page()).evaluate(async () => {
      const { root, steps, show } = window.reply
      const text = 'The first paragraph ends here.\n\nThe second one begins.\n\nAnd a third'
      const at = performance.now()
      steps.value = [{ start: 20, end: text.length, at }]
      await show(text)
      const first = root.children[0]!
      return {
        settled: root.children.length,
        firstFading: getComputedStyle(first.querySelector('.stream-ink')!).opacity !== '1',
        stacked: root.querySelectorAll('.stream-ink .stream-ink').length,
      }
    })
    expect(result).toEqual({ settled: 3, firstFading: true, stacked: 0 })
  })

  // Characters every regular expression of the page scans for the step after
  // a reply's text, counted by their own `exec`: the same for a reply twice
  // as long, as a step lexes and renders the reply's last blocks. Before,
  // each step rendered the whole reply.
  test('a step of a streaming reply renders its last blocks, whatever its length', async () => {
    const counts = await (await page()).evaluate(async (replies) => {
      const { show } = window.reply
      const exec = RegExp.prototype.exec
      const scanned: number[] = []
      for (const reply of replies) {
        await show(`${reply}\n\nThe last para`)
        let count = 0
        RegExp.prototype.exec = function (this: RegExp, input: string) {
          const from = this.global || this.sticky ? this.lastIndex : 0
          const result = exec.call(this, input)
          count += (result ? result.index + result[0].length : input.length) - from
          return result
        }
        try {
          await show(`${reply}\n\nThe last paragraph goes on`)
        } finally {
          RegExp.prototype.exec = exec
        }
        scanned.push(count)
        // A new reply, which starts over.
        await show('')
      }
      return scanned
    }, [essay(100), essay(200)])
    expect(counts[1]).toBe(counts[0]!)
  })
})
