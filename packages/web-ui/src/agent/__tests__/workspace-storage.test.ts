import { afterEach, expect, spyOn, test } from 'bun:test'
import { readWorkspace } from '../workspace-storage'
import { AgentWorkspace } from '../workspace'
import { ConversationRuntime } from '../conversation-runtime'
import type { ControlApi } from '../../transport/protocol'

const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'localStorage')
afterEach(() => {
  if (descriptor) {
    Object.defineProperty(globalThis, 'localStorage', descriptor)
  } else {
    Reflect.deleteProperty(globalThis, 'localStorage')
  }
})
const model = {
  providerId: 'test',
  modelId: 'test',
  thinkingEffort: null,
  serviceTierId: null,
}
const first = {
  id: 'first',
  title: 'First',
  createdAt: '2026-09-01T00:00:00Z',
  model,
}
const control: ControlApi = {
  listProviders: async () => [],
  listModels: async () => [],
  defaultWorkspace: async () => ({ cwd: '/work' }),
  prepareSession: async () => {
    throw new Error('No real model connection in this test')
  },
}

test('saved workspace rejects malformed records and inconsistent references as a whole', () => {
  expect(readWorkspace({ getItem: () => null }, 'key')).toBeNull()
  for (const value of [
    [],
    null,
    { activeId: 'missing', conversations: [first] },
    { activeId: 'first', conversations: [first, first] },
    {
      activeId: 'first',
      conversations: [{ ...first, model: { providerId: 3 } }],
    },
  ]) {
    expect(() =>
      readWorkspace({ getItem: () => JSON.stringify(value) }, 'key'),
    ).toThrow('Invalid saved workspace')
  }
})

test('restoration validates before creating runtimes and preserves the ordered array on save', async () => {
  let raw = '[]'
  let writes = 0
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: () => raw,
      setItem: (_key: string, value: string) => {
        raw = value
        writes += 1
      },
    },
  })
  const connection = spyOn(
    ConversationRuntime.prototype,
    'connect',
  ).mockResolvedValue()
  const workspace = new AgentWorkspace({
    baseUrl: 'http://fixture',
    control,
    cwd: '/work',
  })
  try {
    await expect(workspace.init()).rejects.toThrow('Invalid saved workspace')
    expect(workspace.tabs.value).toEqual([])
    expect(connection).not.toHaveBeenCalled()
    expect(writes).toBe(0)
    raw = JSON.stringify({ activeId: 'first', conversations: [first] })
    await workspace.init()
    expect(workspace.tabs.value.map((tab) => tab.id)).toEqual(['first'])
    workspace.renameConversation('first', 'Renamed')
    expect(JSON.parse(raw)).toEqual({
      activeId: 'first',
      conversations: [{ ...first, title: 'Renamed' }],
    })
    expect(() => workspace.reorderTabs(['missing'])).toThrow()
  } finally {
    await workspace.dispose()
    connection.mockRestore()
  }
})
