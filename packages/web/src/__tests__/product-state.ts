import type { z } from 'zod'
import { productStateSchema, type ConversationSummary, type ProductState } from '../api/generated/web-api'

/**
 * A product state as the channel's snapshot carries it, for the page's tests: a master
 * of a shared instance with no providers, workspaces, devices or
 * conversations, no skill source, whose Cloud is not made yet. Each
 * of `parts` replaces a whole top-level field.
 */
export function productState(parts: Partial<z.input<typeof productStateSchema>> = {}): ProductState {
  return productStateSchema.parse({
    user: {
      id: 'user',
      email: 'test@example.test',
      nickname: 'Test',
      role: 'master',
      createdAt: '2026-09-10T00:00:00.000Z',
    },
    mode: 'shared',
    preferences: { appearance: {}, shortcuts: {} },
    providers: [],
    workspaces: [],
    devices: [],
    publicUrl: 'http://127.0.0.1:3271/',
    conversations: [],
    cloud: {
      device: null,
      state: 'unallocated',
      operation: null,
      error: null,
      volumes: null,
      limits: { systemBytes: 16 * 1024 ** 3, homeBytes: 32 * 1024 ** 3 },
      newerImage: false,
    },
    subagents: { enabled: true, profiles: [] },
    plugins: [{ id: 'skills', name: 'Skills', description: 'Skills the agent follows.', enabled: true, packages: [] }],
    pluginStates: { skills: { sources: [] } },
    mail: false,
    run: 'run-1',
    ...parts,
  })
}

/** An idle Cloud conversation's summary as the channel carries it, with `parts` replacing its fields. */
export function conversationSummary(id: string, title = '', parts: Partial<ConversationSummary> = {}): ConversationSummary {
  return {
    id, title, pinned: false, archived: false, readRevision: 0, revision: 0, unread: false,
    titleCurrent: true, titleGenerating: false, pluginsChanged: false, draftRevision: 0, panelRevision: 0, hostsRevision: 0, pluginRevisions: [],
    permissionRequests: 0, permissionsRevision: 0, cwd: '/home/demi', target: { kind: 'cloud' },
    contextVersion: 0, model: null, createdAt: '2026-09-09T00:00:00.000Z', updatedAt: '2026-09-09T00:00:00.000Z',
    status: 'idle', ...parts,
  }
}
