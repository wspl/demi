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
      disabled: false,
      choosesPassword: false,
    },
    mode: 'shared',
    preferences: { appearance: {}, shortcuts: {} },
    providers: [],
    workspaces: [],
    devices: [],
    publicUrl: 'http://127.0.0.1:3271/',
    installCommands: {
      linux: "curl --proto '=http' -sSf 'http://127.0.0.1:3271/install.sh' | sh",
      macos: "curl --proto '=http' -sSf 'http://127.0.0.1:3271/install.sh' | sh",
      windows: "irm 'http://127.0.0.1:3271/install.ps1' | iex",
    },
    stunUrls: [],
    runnerRelease: null,
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
    subagents: {
      enabled: true,
      maxLive: { limit: 10, unlimited: false },
      maxDepth: { limit: 1, unlimited: false },
      profiles: [],
    },
    instructions: '',
    credentials: [],
    passkeys: { passkeys: [], userHandle: 'dXNlci0x' },
    sessions: [],
    passkeyRpId: null,
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
    pluginsChanged: false, draftRevision: 0, saved: 0, savedRevision: 0, panelRevision: 0, hostsRevision: 0, lastAttachment: 0, pluginRevisions: [],
    permissionRequests: 0, unreadPermissionRequests: 0, permissionsRevision: 0, cwd: '/home/demi', target: { kind: 'cloud' },
    contextVersion: 0, model: null, workflow: { type: 'default' }, createdAt: '2026-09-09T00:00:00.000Z', updatedAt: '2026-09-09T00:00:00.000Z',
    status: 'idle', lastTurn: null, ...parts,
  }
}
