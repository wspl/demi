import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { StubProvider, events } from '@demicodes/provider/testing'
import { createRuntime, createSession, text } from './helpers'

test('Fork captures its prefix before awaiting state reconstruction, even when the source is edited', async () => {
  const entered = deferred<void>()
  const release = deferred<void>()
  const session = createSession(new StubProvider([
    [events.text('original answer'), events.response()],
    [events.text('replacement answer'), events.response()],
  ]), createRuntime({
    async restoreState(context) {
      if (context.transcript.blocks.some((block) => block.type === 'text')) {
        entered.resolve()
        await release.promise
        return { toolCalls: 1 }
      }
      return { toolCalls: 0 }
    },
  }))
  try {
    await session.commandStorage().writeJson('value', 1)
    await session.send(text('original question'))
    const answer = session.transcript().blocks.find((block) => block.type === 'text')!
    const prefix = structuredClone(session.transcript().blocks.slice(0, 2))
    const fork = session.prepareFork(answer.id)
    await entered.promise
    await session.commandStorage().writeJson('value', 2)
    await session.editAndSend({
      operationId: 'edit-source', targetBlockId: session.transcript().blocks[0]!.id,
      version: session.transcript().version(), content: text('edited question'),
    })
    await session.waitUntilDone()
    release.resolve()
    const seed = await fork
    expect(seed.transcript.blocks).toEqual(prefix)
    expect(seed.state).toEqual({ toolCalls: 1 })
    expect(seed.commandState.revision).toBe(1)
    expect(seed.queue).toEqual([])
    expect(seed.phase).toBe('idle')
    expect(seed.edits).toBeUndefined()
    expect(session.transcript().blocks.some((block) => block.id === answer.id)).toBe(false)
  } finally {
    release.resolve()
    await session.dispose()
  }
})

test('Fork rejects reconstruction that changes the retained prefix without changing the source', async () => {
  const session = createSession(new StubProvider([[events.text('answer'), events.response()]]), createRuntime({
    restoreState(context) {
      context.transcript.replaceAll([])
      return { toolCalls: 0 }
    },
  }))
  try {
    await session.send(text('question'))
    const before = session.transcript().toJSON()
    const answer = before.blocks.find((block) => block.type === 'text')!
    await expect(session.prepareFork(answer.id)).rejects.toThrow('changed the Fork transcript')
    expect(session.transcript().toJSON()).toEqual(before)
    await expect(session.prepareFork(before.blocks[0]!.id)).rejects.toThrow('completed assistant')
  } finally {
    await session.dispose()
  }
})
