import { expect, test } from 'bun:test'
import { EditorState } from '@codemirror/state'
import { ensureSyntaxTree, highlightingFor } from '@codemirror/language'
import { highlightTree } from '@lezer/highlight'
import { fenceLanguage, loadFileLanguage } from '../../editor/language'
import { editorTheme } from '../../editor/theme/cmTheme'
import { renderMarkdown } from '../render'

// Code shows in one highlighter and theme wherever it shows (`file-previews.md`
// § Code). Cost: two languages loaded and a few lines parsed, under 50 ms.

test('a code block is labeled with the full name of the language its fence names, by name, short name or extension', () => {
  const label = (fence: string) => renderMarkdown(`\`\`\`${fence}\nx\n\`\`\``).match(/<div class="code-block-lang">(.*?)<\/div>/)?.[1] ?? null
  expect(label('ts')).toBe('TypeScript')
  expect(label('typescript')).toBe('TypeScript')
  expect(label('rs')).toBe('Rust')
  expect(label('sh')).toBe('Shell')
  expect(label('bash')).toBe('Shell')
  expect(label('py')).toBe('Python')
  expect(label('yml')).toBe('YAML')
  // An extension the editor reads as a language the data does not match by name, as a file of it is named.
  expect(label('jsonc')).toBe('JSON')
  // One the highlighter does not know shows as the fence wrote it, and plain.
  expect(label('text')).toBe('text')
  expect(renderMarkdown('```text\n<x>\n```')).toContain('<code>&lt;x&gt;</code>')
  // No fence language, no label.
  expect(label('')).toBeNull()
})

/** A message's code block as runs of its text and the classes that color them; unstyled text is left out. */
function messageRuns(fence: string, code: string): [string, string][] {
  const html = renderMarkdown(`\`\`\`${fence}\n${code}\n\`\`\``)
  const unescape = (text: string) => text.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&')
  return [...html.matchAll(/<span class="([^"]+)">([^<]*)<\/span>/g)].map((match) => [unescape(match[2]!), match[1]!])
}

/** The same code in the editor, for a file of that name: the runs its syntax highlighting colors, with the editor's classes. */
async function editorRuns(path: string, code: string): Promise<[string, string][]> {
  const state = EditorState.create({ doc: code, extensions: [editorTheme(), await loadFileLanguage(path)] })
  const tree = ensureSyntaxTree(state, state.doc.length, 5_000)!
  const runs: [string, string][] = []
  highlightTree(tree, { style: (tags) => highlightingFor(state, tags) }, (from, to, classes) => {
    runs.push([code.slice(from, to), classes])
  })
  return runs
}

test.each([
  ['rust', 'main.rs', 'fn main() {\n    let count: u32 = 42; // the answer\n    println!("{count} <ok>");\n}'],
  ['bash', 'build.sh', 'for f in *.rs; do\n  echo "$f" | grep -c fn  # count\ndone'],
])('a %s code block in a message is colored with the classes the editor gives the same code', async (fence, path, code) => {
  // The message highlights once the fence's language has loaded.
  await fenceLanguage(fence)!.load()
  const message = messageRuns(fence, code)
  expect(new Set(message.map(([, classes]) => classes)).size).toBeGreaterThanOrEqual(3)
  expect(message).toEqual(await editorRuns(path, code))
})
