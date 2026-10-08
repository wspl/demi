import { describe, expect, test } from 'bun:test'
import type { Block, EditedFile } from '@demicodes/protocol'
import type { ToolCallBlock } from '../../agent/block-types'
import { createdAt, model, userBlock } from '../../agent/__tests__/agent-harness'
import {
  editIndex,
  offersAllChanges,
  findRequest,
  pillSelection,
  requestLineSelection,
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

function shellCall(id: string, description: string, files: EditedFile[]): ToolCallBlock {
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
      shellId: 'shell',
      commandId: id,
      exitCode: 0,
      runningMs: 10,
      idleMs: 0,
      chunks: [],
      viewTruncated: false,
      files,
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
    expect({ added: login.added, removed: login.removed }).toEqual({ added: 3, removed: 3 })
    expect(requestLineSelection(null, fix)).toEqual({ node: null, request: 'u1', file: '/w/login.ts', edit: null })
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
    const edit = (segment: number, copies?: ReturnType<typeof kept>) => ({ call: 'c', title: 'Edit', segment, created: false, ...(copies ? { copies } : {}) })
    const firstLost = { path: '/w/a.ts', kind: 'modified' as const, added: 2, removed: 2, edits: [edit(0), edit(1, kept(2)), edit(2, kept(3))] }
    expect(offersAllChanges(firstLost)).toBe(false)
    expect(editIndex(firstLost, null)).toBe(1)
    const nothingKept = { ...firstLost, edits: [edit(0), edit(1)] }
    expect(editIndex(nothingKept, null)).toBe(0)
    expect(selectionCopies(nothingKept, editIndex(nothingKept, null))).toBeNull()
    expect(offersAllChanges(login)).toBe(true)
    expect(editIndex(login, null)).toBeNull()
  })

  test('a pill selects its file at that call’s first edit', () => {
    const requests = transcriptRequests(transcript)
    const selection = pillSelection('child', requests, second, '/w/login.ts')
    expect(selection).toEqual({ node: 'child', request: 'u1', file: '/w/login.ts', edit: { call: 'c2', segment: 0 } })
    const at = editIndex(login, selection!.edit)
    expect(at).toBe(2)
    expect(selectionCopies(login, at)).toEqual({ original: blob(3), modified: blob(4) })
    // The first call's pill starts at its first segment, not its last.
    expect(editIndex(login, pillSelection(null, requests, first, '/w/login.ts')!.edit)).toBe(0)
  })
})
