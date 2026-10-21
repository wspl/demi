import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { Marked } from 'marked'
import { languages } from '@codemirror/language-data'
import { fenceLanguage } from '../../editor/language'
import { REGION_WAIT_MS, resetRegionWaits } from '../../infra/region-wait'
import { codeBlockHtml, codeColor, codeRuns, codeTextColor, loadHighlighter, loadHighlighterWithCommonLanguages } from '../highlight'
import { loadKatex, loadedKatex, mathExtensions } from '../math'
import { COMMON_FENCES, fenceLabel } from '../code-labels'
import { renderMarkdown } from '../render'

// Code and math in a reply draw once (`docs/decisions/0033-a-region-draws-once.md`):
// shown after the background load they draw in their final form, shown while
// their renderer loads they wait hidden for 300 ms at most and then draw
// plain, and a render is never made in the plain form while the wait lasts.
// Fake timers; the highlighter, KaTeX and two small languages loaded: under
// 100 ms.

beforeEach(() => {
  jest.useFakeTimers()
  resetRegionWaits()
})
afterEach(() => {
  jest.useRealTimers()
})

/**
 * A language no test of the process has loaded yet, as bun runs the test
 * files in one process, whose load waits until `arrive` is called: its fence.
 */
function gatedLanguage(): { fence: string; arrive: () => Promise<void> } {
  const fence = ['haskell', 'lua', 'erlang', 'ocaml', 'perl', 'pascal', 'fortran', 'scheme'].find((name) => !fenceLanguage(name)?.support)
  const description = fence ? fenceLanguage(fence) : null
  if (!fence || !description)
    throw new Error('every language the test knows is loaded')
  const load = description.load.bind(description)
  let open: () => void = () => {}
  const gate = new Promise<void>((resolve) => {
    open = resolve
  })
  description.load = async () => {
    await gate
    return load()
  }
  return {
    fence,
    arrive: async () => {
      open()
      await description.load()
    },
  }
}

/** Whether a code block's text is hidden: the colour it is drawn in. */
const hidden = (html: string) => html.includes('color: transparent')
/** Whether a code block's text carries syntax colors. */
const colored = (html: string) => html.includes('<span class="')
/** The label a code block shows. */
const label = (html: string) => /<div class="code-block-lang">(.*?)<\/div>/.exec(html)?.[1]

// Keep this the file's first test: it asserts what shows before anything of
// the highlighter has loaded, which the file's other tests load.
test('a code block shows its language name from its first render, before the highlighter has loaded', () => {
  expect(label(codeBlockHtml('x', 'haskell'))).toBe('Haskell')
  expect(label(codeBlockHtml('SELECT 1', 'sql'))).toBe('SQL')
  expect(label(codeBlockHtml('x', 'cpp'))).toBe('C++')
  expect(label(codeBlockHtml('x', 'nolanguage'))).toBe('nolanguage')
})

test('code shown after the background load draws once, in its colors', async () => {
  await loadHighlighterWithCommonLanguages()
  const html = codeBlockHtml('const answer: number = 42', 'ts')
  expect(hidden(html)).toBe(false)
  expect(colored(html)).toBe(true)
  expect(codeBlockHtml('const answer: number = 42', 'ts')).toBe(html)
})

test('code shown while its language loads waits hidden and draws once if the language arrives within 300 ms', async () => {
  await loadHighlighter()
  const { fence, arrive } = gatedLanguage()
  const code = 'main = putStrLn "hi"'
  const first = codeBlockHtml(code, fence)
  expect(hidden(first)).toBe(true)
  expect(colored(first)).toBe(false)
  jest.advanceTimersByTime(REGION_WAIT_MS - 1)
  expect(hidden(codeBlockHtml(code, fence))).toBe(true)
  await arrive()
  const drawn = codeBlockHtml(code, fence)
  expect(hidden(drawn)).toBe(false)
  expect(colored(drawn)).toBe(true)
})

test('code whose language is later than 300 ms draws plain at 300 ms, then changes once when it arrives', async () => {
  await loadHighlighter()
  const { fence, arrive } = gatedLanguage()
  const code = 'print("hi")'
  expect(hidden(codeBlockHtml(code, fence))).toBe(true)
  jest.advanceTimersByTime(REGION_WAIT_MS)
  const plain = codeBlockHtml(code, fence)
  expect(hidden(plain)).toBe(false)
  expect(colored(plain)).toBe(false)
  await arrive()
  expect(colored(codeBlockHtml(code, fence))).toBe(true)
})

/** Math through the page's syntax with the KaTeX `katex` answers. */
function math(katex: () => ReturnType<typeof loadedKatex>, tex: string): string {
  return new Marked({ extensions: mathExtensions({}, katex) }).parse(tex) as string
}

test('math shown while KaTeX loads waits hidden, and draws once when it arrives within 300 ms', async () => {
  await loadKatex()
  const katex = loadedKatex()!
  const waiting = math(() => null, '$$x^2$$')
  expect(waiting).toBe('<p><span class="math math-display math-waiting">x^2</span></p>\n')
  jest.advanceTimersByTime(REGION_WAIT_MS - 1)
  expect(math(() => null, '$$x^2$$')).toContain('math-waiting')
  expect(math(() => katex, '$$x^2$$')).toContain('<span class="katex-display">')
})

test('math not drawn within 300 ms shows its source, then changes once when KaTeX arrives', async () => {
  await loadKatex()
  const katex = loadedKatex()!
  expect(math(() => null, '面积为$x^2$平方米')).toContain('math-waiting')
  jest.advanceTimersByTime(REGION_WAIT_MS)
  expect(math(() => null, '面积为$x^2$平方米')).toBe('<p>面积为<span class="math math-inline">x^2</span>平方米</p>\n')
  expect(math(() => katex, '面积为$x^2$平方米')).toContain('<span class="katex">')
})

test('a reply with a code block and a formula shown after the background load draws final at once', async () => {
  await loadHighlighterWithCommonLanguages()
  await loadKatex()
  const html = renderMarkdown('```ts\nconst a = 1\n```\n\n$$\\sum_i x_i$$\n')
  expect(hidden(html)).toBe(false)
  expect(html).not.toContain('math-waiting')
  expect(colored(html)).toBe(true)
  expect(html).toContain('<span class="katex-display">')
})

test('a code block shows its frame and its language label at once, whatever its text waits for', async () => {
  await loadHighlighter()
  const { fence, arrive } = gatedLanguage()
  const html = codeBlockHtml('main = putStrLn "hi"', fence)
  expect(html).toStartWith('<div class="code-block"><div class="code-block-lang">')
  expect(hidden(html)).toBe(true)
  // The label is the language's full name from the first render, as it stays,
  // whether or not the highlighter's data has loaded.
  expect(label(codeBlockHtml('x', fence))).toBe(fenceLanguage(fence)?.name)
  expect(label(codeBlockHtml('x', fence))).not.toBe(fence)
  await arrive()
  expect(label(codeBlockHtml('x', fence))).toBe(fenceLanguage(fence)?.name)
})

test('the plain form of code stays once shown: the language arriving after the highlighter does not hide it again', async () => {
  await loadHighlighter()
  const { fence, arrive } = gatedLanguage()
  const code = 'main = putStrLn "hi"'
  expect(hidden(codeBlockHtml(code, fence))).toBe(true)
  jest.advanceTimersByTime(REGION_WAIT_MS)
  expect(hidden(codeBlockHtml(code, fence))).toBe(false)
  // The highlighter's own arrival is another render of the same region.
  await loadHighlighter()
  expect(hidden(codeBlockHtml(code, fence))).toBe(false)
  expect(colored(codeBlockHtml(code, fence))).toBe(false)
  await arrive()
  expect(colored(codeBlockHtml(code, fence))).toBe(true)
})

test('the table of language names holds every name, alias and extension the language data resolves a fence by', () => {
  const terms = new Set(['mjs', 'cjs', 'jsonc', 'mdx', 'scss', 'sass', 'less', 'text', 'txt', 'plain', 'nolanguage'])
  for (const language of languages) {
    terms.add(language.name.toLowerCase())
    for (const term of [...language.alias, ...language.extensions])
      terms.add(term.toLowerCase())
  }
  const wrong = [...terms].filter((fence) => fenceLabel(fence) !== (fenceLanguage(fence)?.name ?? fence))
  expect(wrong).toEqual([])
  for (const fence of COMMON_FENCES)
    expect(fenceLanguage(fence)).not.toBeNull()
})

test('code outside a reply, a command row or a message draws by the same region: hidden, then plain, then colored', async () => {
  await loadHighlighter()
  const { fence, arrive } = gatedLanguage()
  const code = 'main = putStrLn "hi"'
  // The text color and the runs the page draws such code in, as the reply's block draws it.
  expect(codeColor(fence)).toBe('transparent')
  expect(codeRuns(code, fence)).toEqual([])
  jest.advanceTimersByTime(REGION_WAIT_MS)
  expect(codeColor(fence)).toBe(codeTextColor())
  expect(codeRuns(code, fence)).toEqual([])
  await arrive()
  expect(codeColor(fence)).toBe(codeTextColor())
  expect(codeRuns(code, fence).length).toBeGreaterThan(0)
})
