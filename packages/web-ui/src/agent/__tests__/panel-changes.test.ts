import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { PanelTabs, dataChanges, type PanelAnswer, type PanelChange, type PanelRead } from '../panel-changes'
import type { PanelTab } from '../panel-tabs'

const page = (id: string, url = `https://${id}.test/`): PanelTab => ({ id, kind: 'page', data: { url } })

/**
 * A backend whose reads answer what the test says and whose requests wait
 * for the test's answer, so the test decides in what order things arrive.
 */
function backend(first: PanelRead) {
  let panel = first
  let reads = 0
  const sent: PanelChange[][] = []
  const answers: ReturnType<typeof deferred<PanelAnswer>>[] = []
  const refused: unknown[] = []
  const tabs = new PanelTabs(
    {
      read: async () => {
        reads += 1
        return panel
      },
      send: (changes) => {
        sent.push([...changes])
        const answer = deferred<PanelAnswer>()
        answers.push(answer)
        return answer.promise
      },
    },
    (error) => void refused.push(error),
  )
  return {
    tabs,
    /** Each request, as the changes it carried. */
    sent,
    refused,
    reads: () => reads,
    /** The backend's panel from now on. */
    set: (next: PanelRead) => void (panel = next),
    /** Answers the oldest request still waiting: changed, unless the test says not. */
    answer: async (revision: number, changed = true) => {
      answers.shift()!.resolve({ revision, changed })
      await settled()
    },
    refuse: async (error: Error) => {
      answers.shift()!.reject(error)
      await settled()
    },
  }
}

async function settled(): Promise<void> {
  for (let turn = 0; turn < 10; turn++) {
    await Promise.resolve()
  }
}

const ids = (tabs: PanelTabs) => tabs.tabs.value.map((tab) => tab.id)
const ops = (requests: PanelChange[][]) => requests.map((changes) => changes.map((change) => change.type))

test('a change shows at once, and those made while one is on its way go together as the next request', async () => {
  const world = backend({ revision: 1, tabs: [page('a')] })
  world.tabs.start()
  await settled()
  world.tabs.change({ type: 'create', tab: page('b') })
  world.tabs.change({ type: 'update', id: 'b', data: { title: 'B' } })
  world.tabs.change({ type: 'move', id: 'b', index: 0 })
  // All show at once; only the first is on its way.
  expect(world.tabs.tabs.value).toEqual([{ id: 'b', kind: 'page', data: { url: 'https://b.test/', title: 'B' } }, page('a')])
  expect(ops(world.sent)).toEqual([['create']])

  await world.answer(2)
  expect(ops(world.sent)).toEqual([['create'], ['update', 'move']])
  await world.answer(3)
  expect(world.tabs.revision).toBe(3)
  expect(ids(world.tabs)).toEqual(['b', 'a'])
})

test('closing several tabs is one request, and an answer one past the panel held is the panel with it, unread', async () => {
  const world = backend({ revision: 4, tabs: [page('a'), page('b'), page('c')] })
  world.tabs.start()
  await settled()
  const read = world.reads()
  world.tabs.change({ type: 'remove', id: 'a' }, { type: 'remove', id: 'c' })
  expect(ops(world.sent)).toEqual([['remove', 'remove']])
  await world.answer(5)
  expect(world.reads()).toBe(read)
  expect(world.tabs.revision).toBe(5)
  expect(ids(world.tabs)).toEqual(['b'])
})

test('an answer that shows another change came in between is followed by a read', async () => {
  const world = backend({ revision: 1, tabs: [page('a')] })
  world.tabs.start()
  await settled()
  const read = world.reads()
  world.tabs.change({ type: 'create', tab: page('b') })
  // Another page's change came first.
  world.set({ revision: 3, tabs: [page('a'), page('x'), page('b')] })
  await world.answer(3)
  expect(world.reads()).toBe(read + 1)
  expect(ids(world.tabs)).toEqual(['a', 'x', 'b'])
})

test('a request that changed nothing while another change landed reads the panel, though its answer is one past the panel held', async () => {
  const world = backend({ revision: 1, tabs: [page('a'), page('b')] })
  world.tabs.start()
  await settled()
  const read = world.reads()
  world.tabs.change({ type: 'update', id: 'b', data: { title: 'B' } })
  // Another page removed the tab first: the update had nothing to do.
  world.set({ revision: 2, tabs: [page('a')] })
  await world.answer(2, false)
  expect(world.reads()).toBe(read + 1)
  expect(ids(world.tabs)).toEqual(['a'])
})

test('a summary that names the revision of a request on its way reads nothing; one past every answer reads the panel', async () => {
  const world = backend({ revision: 1, tabs: [page('a')] })
  world.tabs.start()
  await settled()
  const read = world.reads()
  world.tabs.change({ type: 'create', tab: page('b') })
  // The summary of the page's own change comes before its answer.
  world.tabs.noticed(2)
  await world.answer(2)
  expect(world.reads()).toBe(read)
  expect(ids(world.tabs)).toEqual(['a', 'b'])

  // A plugin's change of the tab after it.
  world.set({ revision: 3, tabs: [page('a'), { ...page('b'), data: { url: 'https://b.test/', tab: 't3' } }] })
  world.tabs.noticed(3)
  await settled()
  expect(world.reads()).toBe(read + 1)
  expect(world.tabs.tabs.value[1]!.data).toEqual({ url: 'https://b.test/', tab: 't3' })
})

test('a removed tab never comes back from a panel read before its removal reached it', async () => {
  const world = backend({ revision: 1, tabs: [page('a'), page('b')] })
  world.tabs.start()
  await settled()
  world.tabs.change({ type: 'remove', id: 'a' })
  expect(ids(world.tabs)).toEqual(['b'])

  // Another page's change reaches the panel first: a newer panel that still has the tab.
  world.tabs.receive({ revision: 2, tabs: [page('a'), page('b'), page('c')] })
  expect(ids(world.tabs)).toEqual(['b', 'c'])
  // The removal is answered at 3, the panel held with it.
  await world.answer(3)
  expect(ids(world.tabs)).toEqual(['b', 'c'])
  // A read older than the panel the page holds changes nothing.
  world.tabs.receive({ revision: 2, tabs: [page('a'), page('b'), page('c')] })
  expect(ids(world.tabs)).toEqual(['b', 'c'])
})

test('a request the backend refuses leaves, the panel shows as the backend has it, and the refusal is told', async () => {
  const world = backend({ revision: 1, tabs: [page('a')] })
  world.tabs.start()
  await settled()
  world.tabs.change({ type: 'create', tab: page('b') })
  world.tabs.change({ type: 'move', id: 'b', index: 0 })
  expect(ids(world.tabs)).toEqual(['b', 'a'])
  await world.refuse(new Error('panel_full'))
  expect(world.refused).toEqual([new Error('panel_full')])
  // The move of a tab the panel never got does nothing, and is still sent in its turn.
  expect(ids(world.tabs)).toEqual(['a'])
  await world.answer(1, false)
  expect(ids(world.tabs)).toEqual(['a'])
})

test('a tab created again with an id the panel had shows until the answer says nothing changed', async () => {
  const world = backend({ revision: 4, tabs: [] })
  world.tabs.start()
  await settled()
  world.tabs.change({ type: 'create', tab: page('old') })
  expect(ids(world.tabs)).toEqual(['old'])
  await world.answer(4, false)
  expect(ids(world.tabs)).toEqual([])
})

test('nothing is sent before the panel starts, as for a conversation that has no record yet', async () => {
  const world = backend({ revision: 0, tabs: [] })
  world.tabs.change({ type: 'create', tab: page('a') })
  await settled()
  expect(ids(world.tabs)).toEqual(['a'])
  expect(world.sent).toEqual([])
  world.tabs.start()
  await settled()
  expect(ops(world.sent)).toEqual([['create']])
})

test('an update names what changed and each field that went as null', () => {
  expect(dataChanges({ url: 'a', tab: 't1', closed: true }, { url: 'b', tab: 't1' })).toEqual({ url: 'b', closed: null })
  expect(dataChanges({ url: 'a' }, { url: 'a' })).toEqual({})
})
