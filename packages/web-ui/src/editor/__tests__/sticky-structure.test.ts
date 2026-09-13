import { describe, expect, it } from 'bun:test'
import { EditorState } from '@codemirror/state'
import { javascript } from '@codemirror/lang-javascript'
import { getStickyStructureStack } from '../sticky/structure'

function createState(doc: string) {
  return EditorState.create({
    doc,
    extensions: [javascript({ typescript: true })],
  })
}

describe('getStickyStructureStack', () => {
  it('returns outermost-to-innermost sticky rows for nested functions', () => {
    const state = createState([
      'export function outer() {',
      '  const value = 1',
      '  function inner() {',
      '    return value',
      '  }',
      '  return inner()',
      '}',
    ].join('\n'))

    const stack = getStickyStructureStack(state, state.doc.line(4).from)

    expect(stack.map((item) => item.headerLineNumber)).toEqual([1, 3])
  })

  it('includes a nested structure when the top position is at that header line start', () => {
    const state = createState([
      'export function outer() {',
      '  const value = 1',
      '  function inner() {',
      '    return value',
      '  }',
      '  return inner()',
      '}',
    ].join('\n'))

    const stack = getStickyStructureStack(state, state.doc.line(3).from)

    expect(stack.map((item) => item.headerLineNumber)).toEqual([1, 3])
  })

  it('includes multiline top-level declarations when the top position is at the header line start', () => {
    const state = createState([
      'type PreviewState = {',
      '  id: string',
      '}',
      'const previewQueue: PreviewState[] = [',
      "  { id: 'ada' },",
      "  { id: 'grace' },",
      ']',
      'const previewConfig = {',
      '  uppercase: false,',
      '}',
    ].join('\n'))

    const stack = getStickyStructureStack(state, state.doc.line(4).from)

    expect(stack.map((item) => item.headerLineNumber)).toContain(4)
  })

  it('includes multiline top-level object declarations when the top position is at that header line', () => {
    const state = createState([
      'const previewQueue: string[] = [',
      "  'ada',",
      ']',
      'const previewConfig = {',
      '  uppercase: false,',
      '  includeTags: true,',
      '}',
    ].join('\n'))

    const stack = getStickyStructureStack(state, state.doc.line(4).from)

    expect(stack.map((item) => item.headerLineNumber)).toContain(4)
  })

  it('returns an empty stack when the position is not inside a multiline structure', () => {
    const state = createState('const value = 1\nconst other = 2')

    expect(getStickyStructureStack(state, state.doc.line(2).from)).toEqual([])
  })
})
