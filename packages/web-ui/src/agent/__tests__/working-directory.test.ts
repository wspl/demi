import { expect, test } from 'bun:test'
import type { Block, ModelSelection } from '@demicodes/protocol'
import { renderMarkdown } from '../../markdown/render'
import type { MessageFileTarget, MessageFiles } from '../../markdown/types'
import { blockDirectories, childMessageFiles } from '../working-directory'

// Where a child wrote what it wrote (`subagents.md` § Working directory).
// Pure, under a millisecond.

const createdAt = '1970-01-01T00:00:00.000Z'
const model = {} as ModelSelection

function text(id: string): Block {
  return { type: 'text', id, createdAt, model, text: id }
}

function directory(id: string, cwd: string): Block {
  return { type: 'context', id, turnId: id, createdAt, model, source: 'working_directory', text: `Working directory: ${cwd}`, cwd }
}

test('a block is read in the directory the latest change before it names, the directory now when none came before', () => {
  const blocks = [
    text('before-any'),
    directory('first', '/slots/1'),
    text('in-first'),
    // Another source's block names no directory.
    { type: 'context', id: 'target', turnId: 'target', createdAt, model, source: 'execution', text: 'The target changed.' } as Block,
    text('still-first'),
    directory('second', '/slots/2'),
    text('in-second'),
  ]
  const at = blockDirectories(blocks, '/slots/2')
  expect(['before-any', 'in-first', 'still-first', 'in-second'].map((id) => at.get(id))).toEqual([
    '/slots/2', '/slots/1', '/slots/1', '/slots/2',
  ])
})

test('a child’s relative links and images read against its directory, and opening one names that directory', () => {
  const opened: MessageFileTarget[] = []
  const conversation: MessageFiles = {
    cwd: '/work',
    open: (target) => opened.push(target),
  }
  const html = renderMarkdown('[cookie](src/cookie.ts) ![chart](out/chart.png)', {
    files: childMessageFiles(conversation, '/slots/2'),
    hostImages: new Map([['/slots/2/out/chart.png', { state: 'shown', src: '/raw?path=/slots/2/out/chart.png' }]]),
  })
  expect(html).toContain('<a href="/slots/2/src/cookie.ts" class="icon-link" data-file-link>')
  expect(html).toContain('<img src="/raw?path=/slots/2/out/chart.png" alt="chart" data-host-file="/slots/2/out/chart.png" />')
  // The same message of the conversation's own agent reads against the conversation's.
  expect(renderMarkdown('[cookie](src/cookie.ts)', { files: conversation })).toContain('<a href="/work/src/cookie.ts"')

  childMessageFiles(conversation, '/slots/2').open?.({ path: '/slots/2/src/cookie.ts', line: 3 })
  expect(opened).toEqual([{ path: '/slots/2/src/cookie.ts', line: 3, root: '/slots/2' }])
})
