import type { z } from 'zod'
import { productStateSchema, type ConversationMetadata, type ConversationSummary, type ProductState } from '../api/generated/web-api'

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
      newerImage: false,
    },
    logins: [],
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
    automations: [],
    automationsUnread: 0,
    portForwards: [],
    exposedPorts: [],
    models: { providers: [], smallModel: null },
    ...parts,
  })
}

/** An idle Cloud conversation's summary as the channel carries it, with `parts` replacing its fields. */
export function conversationSummary(id: string, title: string | null = null, parts: Partial<ConversationSummary> = {}): ConversationSummary {
  return {
    id, title, pinned: false, archived: false, readRevision: 0, revision: 0, unread: false,
    permissionRequests: 0, unreadPermissionRequests: 0, target: { kind: 'cloud' },
    createdAt: '2026-09-09T00:00:00.000Z', updatedAt: '2026-09-09T00:00:00.000Z',
    status: 'idle', automation: null, ...parts,
  }
}

/**
 * A cost with no ledger row, as `NO_COST` of `@demicodes/web-ui` says it;
 * written out, since tests on a happy-dom page import this module before
 * the page registers, and that module loads Vue's DOM runtime.
 */
const NO_COST = {
  totals: { inputTokens: 0, cacheWriteTokens: 0, cacheReadTokens: 0, outputTokens: 0, price: { total: null, input: null, cacheWrite: null, cacheRead: null, output: null } },
  calls: 0,
}

/** An idle Cloud conversation's metadata as its socket carries it, with `parts` replacing its fields. */
export function conversationMetadata(id: string, parts: Partial<ConversationMetadata> = {}): ConversationMetadata {
  return {
    id, model: null, workflow: { type: 'default' }, cwd: '/home/demi', lastTurn: null, turnSpeed: null, pluginsChanged: false,
    otherCost: NO_COST, saved: { revision: 0, messages: [] }, panel: { revision: 0, tabs: [], states: [] },
    hosts: { revision: 0, hosts: [] }, workflows: { revision: 0, list: { state: 'unknown' } }, lastAttachment: 0, pluginRevisions: [],
    permissions: { revision: 0, requests: [] }, shares: [], ...parts,
  }
}
