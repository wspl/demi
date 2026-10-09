import { expect, test } from 'bun:test'
import { effectScope, nextTick, shallowRef, watch } from 'vue'
import type { ChangeSetSource, ConversationFileService, RequestFile } from '@demicodes/plugin-sdk'
import { firstChangeData, showRequestEdit } from '../data'
import { useChangeSources } from '../sources'

// Cost: reactivity only, under a millisecond.

/** The request's files as one frame of the turn derives them: new objects each time. */
function derived(modified: string): RequestFile[] {
  return [{
    path: 'src/app.ts',
    kind: 'modified',
    edits: [{ call: 'call-1', title: 'Edit src/app.ts', path: 'src/app.ts', segment: 0, created: false, copies: { original: 'a'.repeat(64), modified } }],
  }]
}

test('a frame of the turn that changes nothing of the request gives the Change view nothing new to render', async () => {
  const frame = shallowRef(derived('b'.repeat(64)))
  const uncommitted: ChangeSetSource = { files: [], showSides: () => { throw new Error('not shown') } }
  const files: ConversationFileService = {
    workspace: null,
    root: null,
    changes: uncommitted,
    edit: async () => null,
    request: () => ({ id: 'user-1', files: frame.value }),
    showChanges: () => {},
  }
  const data = showRequestEdit(firstChangeData(), { node: null, request: 'user-1', file: 'src/app.ts', edit: null })
  const scope = effectScope()
  let renders = 0
  const sources = scope.run(() => {
    const sources = useChangeSources(files, () => data)
    watch(sources, () => {
      renders += 1
    })
    return sources
  })!
  const shown = sources.value

  for (let turn = 0; turn < 3; turn += 1) {
    frame.value = derived('b'.repeat(64))
    await nextTick()
  }
  expect(renders).toBe(0)
  expect(sources.value).toBe(shown)

  // The agent edits the file again: the request ends at a new blob.
  frame.value = derived('c'.repeat(64))
  await nextTick()
  expect(renders).toBe(1)
  expect(sources.value.conversation?.files[0]?.edits[0]?.copies?.modified).toBe('c'.repeat(64))
  scope.stop()
})
