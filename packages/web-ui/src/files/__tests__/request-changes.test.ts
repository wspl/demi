import { describe, expect, test } from 'bun:test'
import type { Block, ChangedFile, EditedFile, FileEdit } from '@demicodes/protocol'
import type { ToolCallBlock } from '../../agent/block-types'
import { createdAt, model, userBlock } from '../../agent/__tests__/agent-harness'
import { wholeHistory } from '../../agent/history'
import { groupWork, type WorkGroupBlock } from '../../agent/work-groups'
import {
  callPills,
  changeFile,
  editIndex,
  findRequest,
  offersAllChanges,
  pillSelection,
  requestFileSelection,
  runFiles,
  selectionCopies,
  selectionIsBinary,
  storedRequests,
} from '../request-changes'

// Cost: pure, a few milliseconds for the file.

/** A blob name, as the copies of an edit name the file's sides. */
const blob = (n: number) => n.toString(16).padStart(64, '0')

/** Segment `segment` of the call `call`'s file `path`, from blob `n` to `n + 1`. */
function edit(call: string, path: string, n: number | null, segment = 0, created = false): FileEdit {
  return {
    call,
    path,
    segment,
    ...(created ? { created } : {}),
    ...(n === null ? {} : { copies: { original: blob(n), modified: blob(n + 1) } }),
  }
}

/** A listed file as the backend stores it. */
function listed(path: string, kind: ChangedFile['kind'], edits: FileEdit[], lines?: [number, number], from?: string): ChangedFile {
  return {
    path,
    kind,
    ...(from ? { from } : {}),
    ...(lines ? { lines: { added: lines[0], removed: lines[1] } } : {}),
    edits,
  }
}

/** A shell call of the run `run` as the backend stores it: its own files and its run's and request's lists. */
function shellCall(
  id: string,
  run: string,
  description: string,
  files: EditedFile[],
  lists: { run?: ChangedFile[]; request?: ChangedFile[] } = {},
): ToolCallBlock {
  return {
    type: 'tool_call',
    id,
    run,
    createdAt,
    model,
    toolUseId: `use-${id}`,
    toolName: 'shell',
    input: JSON.stringify({ script: 'true', description }),
    status: 'completed',
    endedAt: createdAt, output: [],
    view: {
      kind: 'shell',
      status: 'exited',
      commandId: id,
      exitCode: 0,
      runningMs: 10,
      idleMs: 0,
      chunks: [],
      viewTruncated: false,
      ...(files.length > 0 ? { files } : {}),
      runFiles: lists.run ?? [],
      ...(lists.request ? { requestFiles: lists.request } : {}),
    },
  }
}

/** A file a call's own view lists. */
function own(path: string, kind: EditedFile['kind'], n: number | null, extra: Partial<EditedFile> = {}): EditedFile {
  return {
    path,
    kind,
    added: 1,
    removed: kind === 'added' ? 0 : 1,
    edits: n === null ? [] : [{ copies: { original: blob(n), modified: blob(n + 1) } }],
    ...extra,
  }
}

const text = (id: string): Block => ({ type: 'text', id, createdAt, model, text: 'Done.' })

/**
 * shell1 creates `notes.md` and edits `login.ts`; shell2 edits `login.ts` again;
 * shell3 writes a scratch file in `/tmp`; shell4 removes `notes.md`, removes
 * `old.css`, which was there before, and renames `a.ts`, all in one run. The run's
 * and the request's lists leave `notes.md` out, as the backend stored them.
 */
const login = (calls: string[]) => listed('/w/login.ts', 'modified', calls.map((call, at) => edit(call, '/w/login.ts', at + 1)), [2, 1])
const s1 = shellCall('s1', 's1', 'Write the notes', [own('/w/notes.md', 'added', 10), own('/w/login.ts', 'modified', 1)], {
  run: [listed('/w/notes.md', 'added', [edit('s1', '/w/notes.md', 10, 0, true)], [1, 0]), login(['s1'])],
  request: [listed('/w/notes.md', 'added', [edit('s1', '/w/notes.md', 10, 0, true)], [1, 0]), login(['s1'])],
})
const s2 = shellCall('s2', 's1', 'Tighten the check', [own('/w/login.ts', 'modified', 2)], {
  run: [listed('/w/notes.md', 'added', [], [1, 0]), login(['s1', 's2'])],
  request: [listed('/w/notes.md', 'added', [], [1, 0]), login(['s1', 's2'])],
})
const s3 = shellCall('s3', 's1', 'Draft a fix', [own('/tmp/fix.py', 'added', 20, { outside: true })], {
  run: [listed('/w/notes.md', 'added', [], [1, 0]), login(['s1', 's2'])],
  request: [listed('/w/notes.md', 'added', [], [1, 0]), login(['s1', 's2'])],
})
const lastLists = [
  login(['s1', 's2']),
  listed('/w/old.css', 'deleted', []),
  listed('/w/b.ts', 'renamed', [], undefined, '/w/a.ts'),
]
const s4 = shellCall('s4', 's1', 'Tidy up', [
  own('/w/notes.md', 'deleted', null, { added: 0, removed: 0 }),
  own('/w/old.css', 'deleted', null, { added: 0, removed: 0 }),
  own('/w/b.ts', 'renamed', null, { added: 0, removed: 0, from: '/w/a.ts' }),
], { run: lastLists, request: lastLists })
const transcript: Block[] = [userBlock('u1', 't1', 'Fix the sign-in page'), s1, s2, s3, s4, text('reply'), userBlock('u2', 't2', 'Thanks')]

describe('the files a request shows', () => {
  test('are the ones its latest call stored, each edit named by its call’s title', () => {
    const { requests, requestOf } = storedRequests(transcript)
    expect(requests.map((request) => request.id)).toEqual(['u1', 'u2'])
    const files = requests[0]!.files
    expect(files.map((file) => [file.path, file.kind, file.from])).toEqual([
      ['/w/login.ts', 'modified', undefined],
      ['/w/old.css', 'deleted', undefined],
      ['/w/b.ts', 'renamed', '/w/a.ts'],
    ])
    expect(files[0]!.lines).toEqual({ added: 2, removed: 1 })
    expect(files[0]!.edits.map((entry) => `${entry.call} ${entry.title}`)).toEqual(['s1 Write the notes', 's2 Tighten the check'])
    expect(requestOf.get('reply')?.id).toBe('u1')
    expect(requests[1]!.files).toEqual([])
  })

  test('a subagent’s are in its own transcript, not the parent’s', () => {
    const childCall = shellCall('child-c', 'child-c', 'Restyle the form', [own('/w/form.css', 'modified', 40)], {
      request: [listed('/w/form.css', 'modified', [edit('child-c', '/w/form.css', 40)], [1, 1])],
    })
    const child: Block[] = [userBlock('child-u', 'ct', 'Check the form styles'), childCall, text('child-reply')]
    const transcripts = { history: wholeHistory(transcript), subagents: [{ id: 'child', history: wholeHistory(child) }] }
    expect(findRequest(transcripts, null, 'child-u')).toBeNull()
    expect(findRequest(transcripts, 'child', 'child-u')!.files.map((file) => file.path)).toEqual(['/w/form.css'])
  })
})

describe('the pills', () => {
  const requests = storedRequests(transcript)

  test('of a call show what it did: removed struck, renamed under the new name, never a file outside the agent’s directory', () => {
    expect(callPills(s1).map((file) => [file.path, file.kind])).toEqual([['/w/notes.md', 'added'], ['/w/login.ts', 'modified']])
    expect(callPills(s3)).toEqual([])
    expect(callPills(s4).map((file) => [file.path, file.kind, file.from])).toEqual([
      ['/w/notes.md', 'deleted', undefined],
      ['/w/old.css', 'deleted', undefined],
      ['/w/b.ts', 'renamed', '/w/a.ts'],
    ])
  })

  test('of a run are its latest call’s list, with the counts it stored', () => {
    expect(runFiles([s1, s2, s3, s4]).map(changeFile)).toEqual([
      { path: '/w/login.ts', kind: 'modified', added: 2, removed: 1 },
      { path: '/w/old.css', kind: 'deleted', added: 0, removed: 0 },
      { path: '/w/b.ts', kind: 'renamed', from: '/w/a.ts', added: 0, removed: 0 },
    ])
    expect(runFiles([s1]).map((file) => file.path)).toEqual(['/w/notes.md', '/w/login.ts'])
  })

  test('open a file at the call’s first edit under its name now; a file a later call removed is no control', () => {
    expect(pillSelection('child', requests, s2, 's2', '/w/login.ts', '/slots/2')).toEqual({
      node: 'child',
      request: 'u1',
      file: '/w/login.ts',
      edit: { call: 's2', path: '/w/login.ts', segment: 0 },
      // The view opens for the directory the child wrote the call in.
      root: '/slots/2',
    })
    // The conversation's own agent names none: the view opens for the conversation's.
    expect(pillSelection(null, requests, s2, 's2', '/w/login.ts', null)).not.toHaveProperty('root')
    expect(pillSelection(null, requests, s1, 's1', '/w/notes.md', null)).toBeNull()
    expect(pillSelection(null, requests, s4, 's4', '/w/old.css', null)).toBeNull()
  })

  test('the "+N Files" pill opens a file at All Changes, even one the request removed, which a pill alone is no control for', () => {
    expect(requestFileSelection(null, requests, s4, '/w/old.css', null)).toEqual({ node: null, request: 'u1', file: '/w/old.css', edit: null })
    expect(requestFileSelection('child', requests, s4, '/w/login.ts', '/slots/2')).toEqual({
      node: 'child',
      request: 'u1',
      file: '/w/login.ts',
      edit: null,
      root: '/slots/2',
    })
    // A file the request created and removed is in no list: nothing to open.
    expect(requestFileSelection(null, requests, s4, '/w/notes.md', null)).toBeNull()
  })

  test('of a page read in light form are those of the whole transcript', () => {
    // A page's light form keeps the lists only on the latest call of each
    // run and request.
    const light = transcript.map((block) => {
      if (block.type !== 'tool_call' || block.view?.kind !== 'shell' || block.id === 's4')
        return block
      const { runFiles: _run, requestFiles: _request, ...view } = block.view
      return { ...block, view }
    })
    const shown = (blocks: Block[]) => groupWork(blocks, false)
      .filter((row): row is WorkGroupBlock => row.type === 'work_group')
      .map((group) => runFiles(group.steps.filter((step): step is ToolCallBlock => step.type === 'tool_call')).map(changeFile))
    expect(shown(light)).toEqual(shown(transcript))
    expect(storedRequests(light).requests).toEqual(storedRequests(transcript).requests)
  })
})

describe('the Change view on a request’s file', () => {
  const file = storedRequests(transcript).requests[0]!.files[0]!

  test('All Changes takes the first edit’s original and the last edit’s result', () => {
    expect(selectionCopies(file, null)).toEqual({ original: blob(1), modified: blob(3) })
  })

  test('All Changes is offered only when both its ends were kept; otherwise the file opens at its first edit with contents', () => {
    const firstLost = { ...file, edits: [{ ...edit('c', '/w/a.ts', null), title: 'Edit' }, { ...edit('c', '/w/a.ts', 2, 1), title: 'Edit' }] }
    expect(offersAllChanges(firstLost)).toBe(false)
    expect(editIndex(firstLost, null)).toBe(1)
    const nothingKept = { ...file, edits: [{ ...edit('c', '/w/a.ts', null), title: 'Edit' }] }
    expect(editIndex(nothingKept, null)).toBe(0)
    expect(selectionCopies(nothingKept, editIndex(nothingKept, null))).toBeNull()
    expect(editIndex(file, null)).toBeNull()
  })
})

describe('an edit whose contents were not kept', () => {
  // Planted defect this catches: binary read from the file's name, so a ZIP
  // or an extensionless picture says its change was simply not kept.
  test('says it is binary by the backend’s mark, whatever the file is called', () => {
    const file = (edits: FileEdit[]) => ({ path: 'release', kind: 'added' as const, edits: edits.map((edit) => ({ ...edit, title: 'Pack it' })) })
    const archive = file([{ ...edit('c-1', 'release', null), binary: true }])
    const huge = file([edit('c-1', 'big.log', null)])
    const mixed = file([edit('c-1', 'notes', 1), { ...edit('c-2', 'notes', null), binary: true }])
    expect([selectionIsBinary(archive, 0), selectionIsBinary(archive, null)]).toEqual([true, true])
    expect([selectionIsBinary(huge, 0), selectionIsBinary(huge, null)]).toEqual([false, false])
    expect([selectionIsBinary(mixed, 0), selectionIsBinary(mixed, 1), selectionIsBinary(mixed, null)]).toEqual([false, true, true])
  })
})
