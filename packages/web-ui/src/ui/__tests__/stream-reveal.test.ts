import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { chromium, type Browser, type Page } from 'playwright'
import {
  alignShown,
  closeOpenInlineMarkdown,
  holdIncompleteMarkdown,
  nextStepEnd,
  renderedLength,
  segmentStreamUnits,
  startOfLast,
  STREAM_PACE,
  stepInterval,
  stepMarks,
  wrapSteps,
} from '../stream-reveal'

// Cost: pure, a few milliseconds for the file.

test('segments cover the source, including CJK and spaces', () => {
  const text = 'Hello, world. 输入壳是 44px。'
  expect(segmentStreamUnits(text).join('')).toBe(text)
})

test('alignShown keeps a matching prefix and snaps a rewrite', () => {
  expect(alignShown('输入壳', '输入壳是胶囊')).toBe('输入壳')
  expect(alignShown('输入壳是胶囊', '输入壳')).toBe('输入壳')
  expect(alignShown('abc', 'xyz')).toBe('')
  expect(alignShown('same', 'same')).toBe('same')
})

const settled = (text: string) => holdIncompleteMarkdown(text).visible

test('a step shows about 22 characters and ends on a word, a CJK word too', () => {
  const english = 'The login test fails because it still expects the old cookie name.'
  const end = nextStepEnd(english, 0, english.length, settled)
  expect(english.slice(0, end)).toBe('The login test fails ')
  const chinese = '登录测试失败，是因为它还在找旧的 cookie 名。上周把辅助函数改了名。'
  const cut = nextStepEnd(chinese, 0, chinese.length, settled)
  const boundaries = new Set(segmentStreamUnits(chinese).map((_, index, units) => units.slice(0, index + 1).join('').length))
  expect(boundaries.has(cut)).toBe(true)
  expect(cut).toBeLessThanOrEqual(STREAM_PACE.stepChars)
})

test('a step never leaves a link half shown: it stops before it, or takes it whole when it starts the step', () => {
  const text = 'It reads [the cookie helper](src/auth/cookie.ts) first.'
  const first = nextStepEnd(text, 0, text.length, settled)
  expect(text.slice(0, first)).toBe('It reads ')
  const second = nextStepEnd(text, first, text.length, settled)
  expect(text.slice(first, second)).toBe('[the cookie helper](src/auth/cookie.ts)')
})

test('steps come faster as text waits, within their bounds', () => {
  expect(stepInterval(1)).toBe(STREAM_PACE.maxStepMs)
  expect(stepInterval(10_000)).toBe(STREAM_PACE.minStepMs)
  // 242 characters waiting: 22 at a time catch up in about the target lag.
  expect(stepInterval(242)).toBe(50)
})

test('a step\'s rendered length leaves out its marks and line breaks, and finds where it starts in the render', () => {
  expect(renderedLength('**bold** and `code`\n- item')).toBe('bold and codeitem'.length)
  expect(renderedLength('```ts\nconst a = 1\n```')).toBe('const a = 1'.length)
  expect(renderedLength('| a | b |\n|---|:-:|\n| c | d |\n\n---\n\na | b')).toBe('abcda | b'.length)
  expect(startOfLast('ab\ncd', 3)).toBe(1)
})

/** A deterministic source of numbers in [0, 1), so a failing case is found again. */
function seeded(seed: number): () => number {
  let state = seed
  return () => {
    state = (state + 0x6d2b79f5) | 0
    let t = Math.imul(state ^ (state >>> 15), 1 | state)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

/** Markdown of every construct `renderedLength` reads, in random order and indents. */
function randomMarkdown(random: () => number): string {
  const pick = <T>(items: readonly T[]): T => items[Math.floor(random() * items.length)]!
  const words = ['alpha', 'beta', 'x', '**bold**', '*it*', '_em_', '`code`', '~~gone~~', '[link](https://e.x/a)', '[a](x "t\nu")', '[', ']', '|', '(', ')', '#', '>', '-', '1.']
  const phrase = () => Array.from({ length: 1 + Math.floor(random() * 5) }, () => pick(words)).join(pick([' ', ' ', '']))
  const lines = [
    () => phrase(),
    () => `${'#'.repeat(1 + Math.floor(random() * 7))} ${phrase()}`,
    () => `${pick(['> ', '>', '> > '])}${phrase()}`,
    () => `${pick(['- ', '* ', '+ ', '1. ', '12) ', '- - '])}${phrase()}`,
    () => pick(['```', '```ts', '~~~', '````md', '``']),
    () => pick(['---', '-----', '- - -', '***']),
    () => pick(['| a | b |', '|---|:-:|', '---|---', '| c | d |', 'c | d', '|:--|', `| ${phrase()} | ${phrase()} |`]),
    () => '',
  ]
  return Array.from({ length: 1 + Math.floor(random() * 30) }, () => ' '.repeat(Math.floor(random() * 5)) + pick(lines)()).join('\n')
}

// Cost: a thousand random replies, about 20 ms.
test('a step\'s extent read from its line on is the extent read from the reply\'s start', () => {
  const random = seeded(7)
  for (let sample = 0; sample < 1000; sample += 1) {
    const text = randomMarkdown(random)
    if (!text)
      continue
    const starts = [...new Set(Array.from({ length: 1 + Math.floor(random() * 6) }, () => Math.floor(random() * text.length)))].toSorted((a, b) => a - b)
    const steps = starts.map((start, index) => ({ start, end: starts[index + 1] ?? text.length, at: index }))
    // The extent as the whole reply less what comes before the step.
    const total = renderedLength(text)
    const expected = steps.toReversed().map((step) => ({ fromEnd: total - renderedLength(text.slice(0, step.start)), at: step.at }))
    expect({ text, steps, marks: stepMarks(text, steps) }).toEqual({ text, steps, marks: expected })
  }
})

test('holdIncompleteMarkdown parks a block\'s opening line until its first text', () => {
  const held = (text: string) => holdIncompleteMarkdown(text).held
  expect(held('Intro\n\n- ')).toBe('- ')
  expect(held('- one\n  - ')).toBe('  - ')
  expect(held('Intro\n\n1. ')).toBe('1. ')
  expect(held('Intro\n\n> ')).toBe('> ')
  expect(held('> - ')).toBe('> - ')
  expect(held('Intro\n\n```ts')).toBe('```ts')
  expect(held('Intro\n\n```ts\n')).toBe('```ts\n')
  expect(held('~~~\n')).toBe('~~~\n')
  expect(held('- one')).toBe('')
  expect(held('```ts\nconst')).toBe('')
  expect(held('Intro\n\n---')).toBe('')
  expect(held('Version 1.')).toBe('')
})

test('holdIncompleteMarkdown parks unmatched markers and links', () => {
  expect(holdIncompleteMarkdown('hello **')).toEqual(
    { visible: 'hello ', held: '**' }
  )
  expect(holdIncompleteMarkdown('hello **bold**')).toEqual(
    { visible: 'hello **bold**', held: '' }
  )
  expect(holdIncompleteMarkdown('see `')).toEqual({ visible: 'see ', held: '`' })
  expect(holdIncompleteMarkdown('go [docs')).toEqual(
    { visible: 'go ', held: '[docs' }
  )
  // Its brackets would show until the target arrives and makes it a link.
  expect(holdIncompleteMarkdown('go [docs]')).toEqual({ visible: 'go ', held: '[docs]' })
  expect(holdIncompleteMarkdown('go [docs] now').held).toBe('')
  expect(holdIncompleteMarkdown('go [docs](https://ex')).toEqual(
    {
      visible: 'go ',
      held: '[docs](https://ex'
    }
  )
})

test('closeOpenInlineMarkdown closes what the last paragraph left open', () => {
  expect(closeOpenInlineMarkdown('2. **Work')).toBe('**')
  expect(closeOpenInlineMarkdown('2. **Work through** one')).toBe('')
  expect(closeOpenInlineMarkdown('a *b **c')).toBe('***')
  expect(closeOpenInlineMarkdown('use `code')).toBe('`')
  expect(closeOpenInlineMarkdown('use `**not bold')).toBe('`')
  expect(closeOpenInlineMarkdown('done **here**\n\n* item\n* other')).toBe('')
  expect(closeOpenInlineMarkdown('2 * 3 = 6')).toBe('')
  expect(closeOpenInlineMarkdown('```ts\nconst a = 1')).toBe('')
  expect(closeOpenInlineMarkdown('~~gone')).toBe('~~')
})

test('closeOpenInlineMarkdown reads no emphasis in math, written or still being written, and none in code', () => {
  // A subscript's underscore, in a formula that is closed.
  expect(closeOpenInlineMarkdown('Each price $p_i$ counts once.')).toBe('')
  expect(closeOpenInlineMarkdown('Both $a_1$ and $b_2$ count.')).toBe('')
  expect(closeOpenInlineMarkdown('The product is $a*b$.')).toBe('')
  expect(closeOpenInlineMarkdown('Sum: $$x_i$$ here')).toBe('')
  // A formula the stream has not closed yet, inline and as a block.
  expect(closeOpenInlineMarkdown('The sum is $\\sum_{i=1}')).toBe('')
  expect(closeOpenInlineMarkdown('Then:\n\n$$\n\\sum_i x_i')).toBe('')
  // Emphasis around math, and prices, still close.
  expect(closeOpenInlineMarkdown('**bold $a_b$ text')).toBe('**')
  expect(closeOpenInlineMarkdown('it costs $10 and _more')).toBe('_')
  // Dollar signs in code are no math, and the code still closes.
  expect(closeOpenInlineMarkdown('use `$HOME_DIR` here')).toBe('')
  expect(closeOpenInlineMarkdown('use `$HOME_DIR')).toBe('`')
})

// A step's fade in a real Chromium, since happy-dom computes no animated
// style: about 1 s for the file, the browser's start, shared by its tests.
describe.skipIf(!existsSync(chromium.executablePath()))('in Chromium', () => {
  let browser: Browser
  let script: string
  beforeAll(async () => {
    const build = await Bun.build({
      entrypoints: ['/entry.ts'],
      files: { '/entry.ts': `import { wrapSteps } from ${JSON.stringify(fileURLToPath(new URL('../stream-reveal.ts', import.meta.url)))}\nObject.assign(window, { wrapSteps })` },
      target: 'browser',
    })
    script = await build.outputs[0]!.text()
    browser = await chromium.launch()
  })
  afterAll(async () => {
    await browser?.close()
  })

  async function page(): Promise<Page> {
    const opened = await browser.newPage()
    await opened.addScriptTag({ content: script })
    return opened
  }

  // The flash lives in what the engine draws between a step being shown and
  // its fade starting on the timeline.
  test('a step\'s new text shows transparent until its fade starts, never solid first', async () => {
    const opacity = await (await page()).evaluate(() => new Promise<number>((resolve) => {
      const wrap: typeof wrapSteps = Reflect.get(window, 'wrapSteps')
      // A step is shown by the stream's timer, a task after a frame began,
      // so the timeline still reads that frame's time, before the step's.
      requestAnimationFrame(() => setTimeout(() => {
        const message = document.createElement('p')
        message.textContent = 'Seen before. Just shown.'
        document.body.append(message)
        wrap(message, message.textContent, [{ start: 13, end: 24, at: performance.now() }])
        resolve(Number(getComputedStyle(message.querySelector('.stream-ink')!).opacity))
      }))
    }))
    expect(opacity).toBe(0)
  })

  // A render can stay in the page after its fades end, as the blocks of a
  // reply that has settled do: it is then as it was rendered, without the
  // fades' spans or class.
  test('a finished fade leaves the render as it was', async () => {
    const result = await (await page()).evaluate(async (fadeMs) => {
      const wrap: typeof wrapSteps = Reflect.get(window, 'wrapSteps')
      const message = document.createElement('div')
      message.innerHTML = '<p>Run <code>bun test</code> first.</p>\n<ul>\n<li>One step</li>\n<li class="task">Two</li>\n</ul>\n'
      const rendered = message.innerHTML
      document.body.append(message)
      // Shown almost a fade ago, so its fades end in a few milliseconds.
      wrap(message, 'Run `bun test` first.\n\n- One step\n- Two', [{ start: 0, end: 40, at: performance.now() - fadeMs + 20 }])
      const fading = message.querySelectorAll('.stream-ink').length
      await Promise.all(message.getAnimations({ subtree: true }).map((animation) => animation.finished))
      return { fading: fading > 0, same: message.innerHTML === rendered }
    }, STREAM_PACE.fadeMs)
    expect(result).toEqual({ fading: true, same: true })
  })

  // Characters the page's regular expressions scan while `wrapSteps` lays out
  // the fades of a reply's last steps, counted by the regular expressions' own
  // `exec`: the same for a reply twice as long, as a frame reads only the
  // lines its steps are on. Before, each frame read the whole reply once per
  // fading step. Cost: one page, a few milliseconds.
  test('a frame of a streaming reply reads its last lines, whatever its length', async () => {
    const scanned = await (await page()).evaluate(() => {
      const wrap: typeof wrapSteps = Reflect.get(window, 'wrapSteps')
      const exec = RegExp.prototype.exec
      return [200, 400].map((paragraphs) => {
        const lines = Array.from({ length: paragraphs }, (_, n) => `Paragraph ${n} goes on about **the matter** at some length, as an essay does.`)
        const source = lines.join('\n\n')
        const message = document.createElement('div')
        message.innerHTML = lines.map((line) => `<p>${line.replaceAll(/\*\*(.*?)\*\*/g, '<strong>$1</strong>')}</p>`).join('\n')
        document.body.append(message)
        // Eight steps of 22 characters, the last two lines, still fading.
        const now = performance.now()
        const steps = Array.from({ length: 8 }, (_, index) => ({ start: source.length - (8 - index) * 22, end: source.length - (7 - index) * 22, at: now }))
        let count = 0
        RegExp.prototype.exec = function (this: RegExp, input: string) {
          const from = this.global || this.sticky ? this.lastIndex : 0
          const result = exec.call(this, input)
          count += (result ? result.index + result[0].length : input.length) - from
          return result
        }
        try {
          wrap(message, source, steps)
        } finally {
          RegExp.prototype.exec = exec
        }
        return count
      })
    })
    expect(scanned[1]).toBe(scanned[0]!)
  })

  // Each case: a reply's source, its render (as the message renderer writes
  // it), the element that paints a box, rule, marker or underline of its own,
  // and where in the source a step boundary falls inside that element's text.
  const BOXES = [
    { name: 'inline code', source: 'It reads `src/auth/cookie.ts` first.', html: '<p>It reads <code>src/auth/cookie.ts</code> first.</p>', box: 'code', cut: 'cookie.ts' },
    { name: 'a link', source: 'See [the cookie helper](https://example.com) here.', html: '<p>See <a href="https://example.com">the cookie helper</a> here.</p>', box: 'a', cut: 'helper' },
    { name: 'a list item', source: 'Intro\n\n- one item\n- two', html: '<p>Intro</p>\n<ul>\n<li>one item</li>\n<li>two</li>\n</ul>\n', box: 'li', cut: 'item' },
    { name: 'a quote', source: 'Intro\n\n> a quote here', html: '<p>Intro</p>\n<blockquote>\n<p>a quote here</p>\n</blockquote>\n', box: 'blockquote', cut: 'here' },
    {
      name: 'a code block',
      source: 'Intro\n\n```ts\nconst a = 1\nconst b = 2\n```',
      html: '<p>Intro</p>\n<div class="code-block"><div class="code-block-lang">TypeScript</div><pre tabindex="0"><code>const a = 1\nconst b = 2</code></pre></div>',
      box: '.code-block',
      cut: 'const b',
    },
    {
      name: 'a table row',
      source: 'Intro\n\n| a | b |\n|---|---|\n| c | d |',
      html: '<p>Intro</p>\n<div class="table-scroll"><table>\n<thead>\n<tr>\n<th>a</th>\n<th>b</th>\n</tr>\n</thead>\n<tbody><tr>\n<td>c</td>\n<td>d</td>\n</tr>\n</tbody></table>\n</div>\n',
      box: 'tbody tr',
      cut: 'd |',
    },
  ]

  // Cost: one page, a few milliseconds.
  test('an element\'s own paint fades with the first piece of its text, never ahead of it', async () => {
    const seen = await (await page()).evaluate((cases) => {
      const wrap: typeof wrapSteps = Reflect.get(window, 'wrapSteps')
      const now = performance.now()
      /** How opaque `node` reaches the screen: its and its ancestors' opacity together. */
      const opacity = (node: Node) => {
        let product = 1
        for (let element = node instanceof Element ? node : node.parentElement; element && element !== document.body; element = element.parentElement)
          product *= Number(getComputedStyle(element).opacity)
        return product
      }
      const results = cases.map(({ name, source, html, box, cut }) => {
        const message = document.createElement('div')
        message.innerHTML = html
        document.body.append(message)
        // The older step, half faded, shows the element's first characters; the new one the rest.
        const boundary = source.lastIndexOf(cut)
        wrap(message, source, [
          { start: 0, end: boundary, at: now - 200 },
          { start: boundary, end: source.length, at: now },
        ])
        const element = message.querySelector(box)!
        const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT)
        let first = walker.nextNode()
        while (first && !first.textContent?.trim()) first = walker.nextNode()
        return { name, box: opacity(element), text: opacity(first!) }
      })
      const time = Number(document.timeline.currentTime)
      return { results, older: (time - (now - 200)) / 400 }
    }, BOXES)
    // In hundredths: the element and its first characters, both as far into their step's fade.
    const percent = (opacity: number) => Math.round(opacity * 100)
    expect(seen.results.map(({ name, box, text }) => ({ name, box: percent(box), text: percent(text) })))
      .toEqual(seen.results.map(({ name }) => ({ name, box: percent(seen.older), text: percent(seen.older) })))
  })
})
