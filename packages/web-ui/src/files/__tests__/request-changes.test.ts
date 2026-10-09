import { describe, expect, test } from 'bun:test'
import type { Block, EditedFile, PathChange } from '@demicodes/protocol'
import type { ToolCallBlock } from '../../agent/block-types'
import { createdAt, model, userBlock } from '../../agent/__tests__/agent-harness'
import { diffLineCounts } from '../diff-counts'
import {
  editIndex,
  offersAllChanges,
  findRequest,
  pillSelection,
  callFiles,
  fileLineCounts,
  selectionCopies,
  transcriptRequests,
} from '../request-changes'

// Cost: pure, a few milliseconds for the file.

/** A blob name, as the copies of an edit name the file's sides. */
const blob = (n: number) => n.toString(16).padStart(64, '0')

/** A file a call changed, each segment from blob `n` to `n + 1`. */
function edited(path: string, from: number, segments = 1, kind: EditedFile['kind'] = 'modified'): EditedFile {
  return {
    path,
    kind,
    added: segments,
    removed: segments,
    edits: Array.from({ length: segments }, (_, n) => ({ copies: { original: blob(from + n), modified: blob(from + n + 1) } })),
  }
}

function shellCall(id: string, description: string, files: EditedFile[], pathChanges?: PathChange[]): ToolCallBlock {
  return {
    type: 'tool_call',
    id,
    createdAt,
    model,
    toolUseId: `use-${id}`,
    toolName: 'shell_exec',
    input: JSON.stringify({ script: 'true', description }),
    status: 'completed',
    output: [],
    view: {
      kind: 'shell',
      status: 'exited',
      commandId: id,
      exitCode: 0,
      runningMs: 10,
      idleMs: 0,
      chunks: [],
      viewTruncated: false,
      files,
      ...(pathChanges ? { pathChanges } : {}),
    },
  }
}

const text = (id: string): Block => ({ type: 'text', id, createdAt, model, text: 'Done.' })

/**
 * The design's example: the user asks to fix the sign-in page; the agent
 * edits login.ts twice in one call and form.css, yields until a build
 * ends, and woken, also by a subagent's receipt, takes a steer and edits
 * login.ts again. Then the user asks for something else.
 */
const first = shellCall('c1', 'Fix the sign-in check', [edited('/w/login.ts', 1, 2), edited('/w/form.css', 10, 1, 'added')])
const second = shellCall('c2', 'Tighten the check', [edited('/w/login.ts', 3)])
const third = shellCall('c3', 'Note it in the readme', [edited('/w/README.md', 20), edited('/w/login.ts', 30)])
const transcript: Block[] = [
  userBlock('u1', 't1', 'Fix the sign-in page'),
  first,
  shellCall('build', 'Start the build', []),
  { type: 'wakeup', id: 'wake', turnId: 't2', createdAt, model, placement: 'new_turn' },
  {
    type: 'agent_message',
    id: 'receipt',
    turnId: 't3',
    createdAt,
    model,
    message: {
      id: 'receipt',
      recipientId: 'root',
      timestamp: createdAt,
      content: 'Checked the form.',
      event: { type: 'completion', outcome: 'completed' },
    },
  },
  { type: 'steer', id: 'steer', turnId: 't3', createdAt, model, content: [{ type: 'text', text: 'Keep the old error text.' }] },
  second,
  text('reply'),
  userBlock('u2', 't4', 'Document it'),
  third,
  text('reply-2'),
]

describe('a request', () => {
  test('one spanning a yield wakeup and an agent receipt is one request, with every call’s files', () => {
    const { requests } = transcriptRequests(transcript)
    const fix = requests[0]!
    expect(fix.id).toBe('u1')
    expect(fix.files.map((file) => file.path)).toEqual(['/w/login.ts', '/w/form.css'])
    const login = fix.files[0]!
    expect(login.edits.map((edit) => `${edit.call}:${edit.segment} ${edit.title}`)).toEqual([
      'c1:0 Fix the sign-in check',
      'c1:1 Fix the sign-in check',
      'c2:0 Tighten the check',
    ])
  })

  test('a steer does not split it', () => {
    const { requestOf } = transcriptRequests(transcript)
    expect(requestOf.get('steer')?.id).toBe('u1')
    expect(requestOf.get('c2')?.id).toBe('u1')
    expect(requestOf.get('reply')?.id).toBe('u1')
  })

  test('a new user message starts another, which holds only its own calls’ files', () => {
    const { requests } = transcriptRequests(transcript)
    expect(requests.map((request) => request.id)).toEqual(['u1', 'u2'])
    const docs = requests[1]!
    expect(docs.files.map((file) => file.path)).toEqual(['/w/README.md', '/w/login.ts'])
    expect(docs.files[1]!.edits.map((edit) => edit.call)).toEqual(['c3'])
  })

  test('a subagent’s calls are absent from the parent’s request, and present in its own', () => {
    const child: Block[] = [
      userBlock('child-u', 'ct', 'Check the form styles'),
      shellCall('child-c', 'Restyle the form', [edited('/w/form.css', 40)]),
      text('child-reply'),
    ]
    const transcripts = { blocks: transcript, subagents: [{ id: 'child', blocks: child }] }
    const parent = findRequest(transcripts, null, 'u1')!
    expect(parent.files.find((file) => file.path === '/w/form.css')!.edits.map((edit) => edit.call)).toEqual(['c1'])
    expect(findRequest(transcripts, null, 'child-u')).toBeNull()
    const own = findRequest(transcripts, 'child', 'child-u')!
    expect(own.files.map((file) => `${file.path} ${file.edits.map((edit) => edit.call).join()}`)).toEqual(['/w/form.css child-c'])
  })
})

describe('the Change view on a request', () => {
  const login = transcriptRequests(transcript).requests[0]!.files[0]!

  test('All Changes takes the first edit’s original and the last edit’s result', () => {
    expect(selectionCopies(login, null)).toEqual({ original: blob(1), modified: blob(4) })
    // A file created by its first edit starts from that edit's empty original.
    const form = transcriptRequests(transcript).requests[0]!.files[1]!
    expect(form.edits[0]!.created).toBe(true)
  })

  test('All Changes is offered only when both its ends were kept; otherwise the file opens at its first edit with contents', () => {
    const kept = (from: number) => ({ original: blob(from), modified: blob(from + 1) })
    const edit = (segment: number, copies?: ReturnType<typeof kept>) => ({ call: 'c', title: 'Edit', path: '/w/a.ts', segment, created: false, ...(copies ? { copies } : {}) })
    const firstLost = { path: '/w/a.ts', kind: 'modified' as const, edits: [edit(0), edit(1, kept(2)), edit(2, kept(3))] }
    expect(offersAllChanges(firstLost)).toBe(false)
    expect(editIndex(firstLost, null)).toBe(1)
    const nothingKept = { ...firstLost, edits: [edit(0), edit(1)] }
    expect(editIndex(nothingKept, null)).toBe(0)
    expect(selectionCopies(nothingKept, editIndex(nothingKept, null))).toBeNull()
    expect(offersAllChanges(login)).toBe(true)
    expect(editIndex(login, null)).toBeNull()
  })

  test('the header counts the lines of the diff it shows, not the calls’ sums', () => {
    // A line changed twice across two calls is one line changed from the first original to the last result.
    expect(diffLineCounts('a\nb\nc\n', 'a\nB2\nc\nd\n')).toEqual({ added: 2, removed: 1 })
    expect(diffLineCounts('', 'x\ny\n')).toEqual({ added: 2, removed: 0 })
    expect(diffLineCounts('same\n', 'same\n')).toEqual({ added: 0, removed: 0 })
  })

  test('a file counts its All Changes, not the sum of its calls’, when two calls change one line', async () => {
    // Blob n holds version n of a.ts: one line, x = n.
    const texts = (n: number) => `const x = ${n}\n`
    const read = async (copies: { original: string; modified: string }) => ({
      original: texts(parseInt(copies.original, 16)),
      modified: texts(parseInt(copies.modified, 16)),
    })
    const { requests } = transcriptRequests([
      userBlock('u', 't', 'Set x'),
      shellCall('one', 'Set x to 1', [edited('/w/a.ts', 0)]),
      shellCall('two', 'Set x to 2', [edited('/w/a.ts', 1)]),
    ])
    expect(await fileLineCounts(requests[0]!.files[0]!, read)).toEqual({ added: 1, removed: 1 })
  })

  test('a run of a request’s calls counts its files from its own first edit to its last', async () => {
    const texts = (n: number) => `const x = ${n}\n`
    const read = async (copies: { original: string; modified: string }) => ({
      original: texts(parseInt(copies.original, 16)),
      modified: texts(parseInt(copies.modified, 16)),
    })
    const two = shellCall('two', 'Set x to 2', [edited('/w/a.ts', 1)])
    const three = shellCall('three', 'Set x to 3', [edited('/w/a.ts', 2), edited('/w/b.ts', 7, 1, 'added')])
    const files = callFiles([two, three])
    expect(files.map((file) => [file.path, file.kind])).toEqual([['/w/a.ts', 'modified'], ['/w/b.ts', 'added']])
    expect(selectionCopies(files[0]!, null)).toEqual({ original: blob(1), modified: blob(3) })
    expect(await fileLineCounts(files[0]!, read)).toEqual({ added: 1, removed: 1 })
  })

  test('the button has no counts when a file’s first original was not kept', async () => {
    const lost = shellCall('lost', 'Edit without copies', [{ path: '/w/a.ts', kind: 'modified', added: 1, removed: 1, edits: [{}] }])
    const { requests } = transcriptRequests([userBlock('u', 't', 'Edit'), lost, shellCall('kept', 'Edit again', [edited('/w/a.ts', 5)])])
    let reads = 0
    const read = async () => {
      reads += 1
      return { original: '', modified: '' }
    }
    expect(await fileLineCounts(requests[0]!.files[0]!, read)).toBeNull()
    expect(reads).toBe(0)
  })

  test('a pill selects its file at that call’s first edit', () => {
    const requests = transcriptRequests(transcript)
    const selection = pillSelection('child', requests, second, '/w/login.ts')
    expect(selection).toEqual({ node: 'child', request: 'u1', file: '/w/login.ts', edit: { call: 'c2', path: '/w/login.ts', segment: 0 } })
    const at = editIndex(login, selection!.edit)
    expect(at).toBe(2)
    expect(selectionCopies(login, at)).toEqual({ original: blob(3), modified: blob(4) })
    // The first call's pill starts at its first segment, not its last.
    expect(editIndex(login, pillSelection(null, requests, first, '/w/login.ts')!.edit)).toBe(0)
  })
})

describe('a request across renames and removals', () => {
  /**
   * The design's example: one call writes the new page outside the
   * workspace, the next moves it over the existing page, and a later one
   * makes a scratch file that the last removes.
   */
  const write = shellCall('c1', 'Write the new page', [edited('/tmp/page.ts.new', 0, 1, 'added')])
  const place = shellCall('c2', 'Put the new page in place', [edited('/w/src/page.ts', 10)], [
    { kind: 'renamed', from: '/tmp/page.ts.new', to: '/w/src/page.ts' },
  ])
  const scratch = shellCall('c3', 'Make a scratch file', [edited('/w/scratch.txt', 20, 1, 'added')])
  const clean = shellCall('c4', 'Remove the scratch file', [], [{ kind: 'removed', path: '/w/scratch.txt' }])
  const request = transcriptRequests([userBlock('u', 't', 'Replace the page'), write, place, scratch, clean]).requests[0]!

  test('lists its files as they stand after its latest call', () => {
    expect(request.files.map((file) => [file.path, file.kind])).toEqual([['/w/src/page.ts', 'modified']])
  })

  test('a renamed file keeps its earlier edits, each under the name it was made under', () => {
    const page = request.files[0]!
    expect(page.edits.map((edit) => `${edit.call} ${edit.path} ${edit.title}`)).toEqual([
      'c1 /tmp/page.ts.new Write the new page',
      'c2 /w/src/page.ts Put the new page in place',
    ])
    // All Changes spans the page that was there before to the new one, not
    // the creation of the file that replaced it.
    expect(selectionCopies(page, null)).toEqual({ original: blob(10), modified: blob(11) })
  })

  test('a pill opens its file under its new name, and a removed file’s opens nothing', () => {
    const requests = transcriptRequests([userBlock('u', 't', 'Replace the page'), write, place, scratch, clean])
    const selection = pillSelection(null, requests, write, '/tmp/page.ts.new')
    expect(selection).toEqual({
      node: null,
      request: 'u',
      file: '/w/src/page.ts',
      edit: { call: 'c1', path: '/tmp/page.ts.new', segment: 0 },
    })
    expect(editIndex(requests.requests[0]!.files[0]!, selection!.edit)).toBe(0)
    expect(pillSelection(null, requests, scratch, '/w/scratch.txt')).toBeNull()
  })

  test('a folder moves and goes with everything in it, and a rename onto a listed file merges into it', () => {
    const edit = shellCall('e1', 'Edit the files', [
      edited('/w/old/x.ts', 1),
      edited('/w/old.ts', 3),
      edited('/w/tmp/t1.ts', 5, 1, 'added'),
      edited('/w/tmp/deep/t2.ts', 7, 1, 'added'),
      edited('/w/tmp.ts', 9),
      edited('/w/a.ts', 11),
      edited('/w/b.ts', 13),
    ])
    const move = shellCall('e2', 'Rearrange them', [edited('/w/b.ts', 30)], [
      { kind: 'renamed', from: '/w/old', to: '/w/moved' },
      { kind: 'removed', path: '/w/tmp' },
      { kind: 'renamed', from: '/w/a.ts', to: '/w/b.ts' },
    ])
    const files = transcriptRequests([userBlock('u', 't', 'Tidy up'), edit, move]).requests[0]!.files
    expect(files.map((file) => [file.path, file.kind])).toEqual([
      ['/w/moved/x.ts', 'added'],
      ['/w/old.ts', 'modified'],
      ['/w/tmp.ts', 'modified'],
      ['/w/b.ts', 'modified'],
    ])
    expect(files[3]!.edits.map((entry) => `${entry.call} ${entry.path}`)).toEqual(['e1 /w/b.ts', 'e1 /w/a.ts', 'e2 /w/b.ts'])
    // A work group of the later call alone knows nothing of the earlier one's files.
    expect(callFiles([move]).map((file) => file.path)).toEqual(['/w/b.ts'])
  })

  test('a file made again after its removal starts anew', () => {
    const files = transcriptRequests([
      userBlock('u', 't', 'Redo the notes'),
      shellCall('n1', 'Write notes', [edited('/w/notes.md', 1, 1, 'added')]),
      shellCall('n2', 'Rewrite notes', [edited('/w/notes.md', 40, 1, 'added')], [{ kind: 'removed', path: '/w/notes.md' }]),
    ]).requests[0]!.files
    expect(files.map((file) => `${file.path} ${file.edits.map((entry) => entry.call).join()}`)).toEqual(['/w/notes.md n2'])
  })
})
