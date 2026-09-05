import { expect, test } from 'bun:test'
import { renderMarkdown } from '../render'
import { toggleGfmTask } from '../gfm-task'

const src = `### Checklist

- [x] Hairline is a ring, not a drop
- [ ] Radio is still missing
- [ ] Toast / Banner has no shared exit
`

function checkboxCount(markdown: string): number {
  return renderMarkdown(markdown).split('type="checkbox"').length - 1
}

test('toggles the nth GFM task and leaves the others', () => {
  expect(toggleGfmTask(src, 0)).toContain('- [ ] Hairline is a ring, not a drop')
  expect(toggleGfmTask(src, 1)).toContain('- [x] Radio is still missing')
  expect(toggleGfmTask(src, 2)).toContain('- [x] Toast / Banner has no shared exit')
  expect(toggleGfmTask(src, 1)).toContain('- [x] Hairline is a ring, not a drop')
})

test('ignores an out-of-range index', () => {
  expect(toggleGfmTask(src, 9)).toBe(src)
})

test('a task mark inside a code fence is not a rendered box', () => {
  const fenced = 'Example:\n\n```md\n- [ ] sample\n```\n\n- [ ] real one\n- [ ] real two\n'
  expect(checkboxCount(fenced)).toBe(2)
  expect(toggleGfmTask(fenced, 0)).toBe('Example:\n\n```md\n- [ ] sample\n```\n\n- [x] real one\n- [ ] real two\n')
  expect(toggleGfmTask(fenced, 1)).toBe('Example:\n\n```md\n- [ ] sample\n```\n\n- [ ] real one\n- [x] real two\n')
})

test('a blockquoted task is a rendered box', () => {
  const quoted = '> - [ ] quoted\n\n- [ ] plain\n'
  expect(checkboxCount(quoted)).toBe(2)
  expect(toggleGfmTask(quoted, 1)).toBe('> - [ ] quoted\n\n- [x] plain\n')
  expect(toggleGfmTask(quoted, 0)).toBe('> - [x] quoted\n\n- [ ] plain\n')
})

test('rendered task boxes are live, not disabled', () => {
  const html = renderMarkdown(src)
  expect(html).toContain('type="checkbox"')
  expect(html).toContain('checked=""')
  expect(html).not.toContain('disabled')
})
