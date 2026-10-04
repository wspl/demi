import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { PanelTabs, dataChanges, type PanelChange, type PanelRead } from '../panel-changes'
import type { PanelTab } from '../panel-tabs'

const page = (id: string, url = `https://${id}.test/`): PanelTab => ({ id, kind: 'page', data: { url } })

/**
 * A backend whose reads answer what the test says and whose changes wait
 * for the test's answer, so the test decides in what order things arrive.
 */
function backend(first: PanelRead) {
  let panel = first
  const sent: PanelChange[] = []
  const answers: ReturnType<typeof deferred<{ revision: number }>>[] = []
  const refused: unknown[] = []
  const tabs = new PanelTabs(
    {
      read: async () => panel,
      send: (change) => {
        sent.push(change)
        const answer = deferred<{ revision: number }>()
        answers.push(answer)
        return answer.promise
      },
    },
    (error) => void refused.push(error),
  )
  return {
    tabs,
    sent,
    refused,
    /** The backend's panel from now on. */
    set: (next: PanelRead) => void (panel = next),
    /** Answers the oldest change still waiting. */
    answer: async (revision: number) => {
      answers.shift()!.resolve({ revision })
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

test('a change shows at once, leaves in order, and stays shown until a panel that has it is read', async () => {
  const world = backend({ revision: 1, tabs: [page('a')] })
  world.tabs.start()
  await settled()
  world.tabs.change({ type: 'create', tab: page('b') })
  world.tabs.change({ type: 'update', id: 'b', data: { title: 'B' } })
  // Both show at once; only the first is on its way.
  expect(world.tabs.tabs.value).toEqual([page('a'), { id: 'b', kind: 'page', data: { url: 'https://b.test/', title: 'B' } }])
  expect(world.sent.map((change) => change.type)).toEqual(['create'])

  // The create is in the panel at revision 2; a read before the backend's next answer still shows both.
  world.set({ revision: 2, tabs: [page('a'), page('b')] })
  await world.answer(2)
  expect(world.sent.map((change) => change.type)).toEqual(['create', 'update'])
  expect(world.tabs.tabs.value[1]!.data).toEqual({ url: 'https://b.test/', title: 'B' })
  world.set({ revision: 3, tabs: [page('a'), { id: 'b', kind: 'page', data: { url: 'https://b.test/', title: 'B' } }] })
  await world.answer(3)
  expect(world.tabs.revision).toBe(3)
  expect(world.tabs.tabs.value[1]!.data).toEqual({ url: 'https://b.test/', title: 'B' })
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
  // The removal is answered at 3, and a read from before it arrives late.
  world.set({ revision: 2, tabs: [page('a'), page('b'), page('c')] })
  await world.answer(3)
  expect(ids(world.tabs)).toEqual(['b', 'c'])
  world.tabs.receive({ revision: 3, tabs: [page('b'), page('c')] })
  expect(ids(world.tabs)).toEqual(['b', 'c'])
  // A read older than the panel the page holds changes nothing.
  world.tabs.receive({ revision: 2, tabs: [page('a'), page('b'), page('c')] })
  expect(ids(world.tabs)).toEqual(['b', 'c'])
})

test('a change the backend refuses leaves, the panel shows as the backend has it, and the refusal is told', async () => {
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
  await world.answer(1)
  expect(ids(world.tabs)).toEqual(['a'])
})

test('a tab created again with an id the panel had shows until the answer says nothing changed', async () => {
  const world = backend({ revision: 4, tabs: [] })
  world.tabs.start()
  await settled()
  world.tabs.change({ type: 'create', tab: page('old') })
  expect(ids(world.tabs)).toEqual(['old'])
  await world.answer(4)
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
  expect(world.sent.map((change) => change.type)).toEqual(['create'])
})

test('an update names what changed and each field that went as null', () => {
  expect(dataChanges({ url: 'a', tab: 't1', closed: true }, { url: 'b', tab: 't1' })).toEqual({ url: 'b', closed: null })
  expect(dataChanges({ url: 'a' }, { url: 'a' })).toEqual({})
})
