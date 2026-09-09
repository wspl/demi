import { z } from 'zod'
import { modelSelectionSchema } from '@demicodes/web-ui/transport/protocol'

export const userSchema = z.object({
  id: z.string().min(1),
  email: z.email(),
  nickname: z.string(),
  role: z.enum(['master', 'admin', 'user']),
  createdAt: z.iso.datetime({ offset: true }),
})
export const identitySchema = z.object({ user: userSchema })

export const appearanceSchema = z.object({
  theme: z.enum(['system', 'light', 'dark']).optional(),
  tone: z.enum(['ink', 'warm']).optional(),
  accent: z
    .enum(['blue', 'purple', 'pink', 'red', 'orange', 'green', 'teal'])
    .optional(),
  fontSize: z.number().int().min(12).max(18).optional(),
})
export const preferencesSchema = z.object({
  appearance: appearanceSchema,
  shortcuts: z.object({
    new: z.string().optional(),
    sidebar: z.string().optional(),
    settings: z.string().optional(),
  }),
})
export type AppearancePatch = z.infer<typeof appearanceSchema>
export type Preferences = z.infer<typeof preferencesSchema>
export type PreferencesPatch = {
  appearance?: AppearancePatch
  shortcuts?: Partial<Record<'new' | 'sidebar' | 'settings', string | null>>
}

export const deviceSchema = z.object({
  id: z.string().min(1),
  name: z.string(),
  kind: z.enum(['user', 'managed']),
  platform: z.string(),
  claimedAt: z.string(),
  lastSeenAt: z.string().nullable(),
  online: z.boolean(),
  home: z.string().nullable(),
})
export const workspaceSchema = z.object({
  id: z.string().min(1),
  deviceId: z.string().min(1),
  path: z.string().min(1),
  name: z.string(),
  createdAt: z.string(),
})
export const targetSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('cloud') }),
  z.object({
    kind: z.literal('device'),
    deviceId: z.string().min(1),
    path: z.string().min(1),
  }),
  z.object({
    kind: z.literal('workspace'),
    workspaceId: z.string().min(1),
  }),
])
export const conversationRecordSchema = z.object({
  id: z.string().min(1),
  title: z.string(),
  archived: z.boolean(),
  pinned: z.boolean(),
  readRevision: z.number().int().nonnegative(),
  target: targetSchema,
  contextVersion: z.number().int().nonnegative(),
  providerId: z.string().nullable(),
  modelId: z.string().nullable(),
  createdAt: z.string(),
  updatedAt: z.string(),
})
export const conversationSummarySchema = conversationRecordSchema.extend({
  status: z.enum([
    'running',
    'compacting',
    'interrupted',
    'error',
    'stopped',
    'completed',
    'idle',
  ]),
  revision: z.number().int().nonnegative(),
  unread: z.boolean(),
})
export const hostsSchema = z.object({
  hosts: z.array(
    z.object({
      deviceId: z.string(),
      name: z.string(),
      cwd: z.string().nullable(),
      online: z.boolean(),
    }),
  ),
})

const healthSchema = z.object({
  status: z.enum([
    'unknown',
    'ready',
    'authenticated',
    'unauthenticated',
    'unavailable',
    'error',
  ]),
  message: z.string().optional(),
})
const credentialSchema = z.object({
  id: z.string(),
  label: z.string(),
  detail: z.string().nullable().optional(),
})
const activeCredentialSchema = z.object({
  credentialId: z.string().nullable(),
  status: healthSchema,
})
export const quotaSchema = z.object({
  observedAt: z.string(),
  accountLabel: z.string().nullable(),
  plan: z
    .object({
      id: z.string().nullable(),
      label: z.string().nullable(),
    })
    .nullable(),
  windows: z.array(
    z.object({
      id: z.string(),
      label: z.string().optional(),
      usedPercent: z.number().nullable(),
      used: z.number().nullable().optional(),
      limit: z.number().nullable().optional(),
      unit: z.string().optional(),
      resetsAt: z.string().nullable(),
    }),
  ),
})
const providerDetailsSchema = z.object({
  auth: healthSchema,
  runtime: healthSchema,
  accounts: z.array(credentialSchema),
  active: activeCredentialSchema.nullable(),
  quota: quotaSchema.nullable(),
  quotaCapability: z.object({
    mode: z.enum(['none', 'supported']),
    canProbe: z.boolean().optional(),
    probeCost: z.enum(['free', 'minimal_request']).optional(),
  }),
  requiresProcessCapableHost: z.boolean(),
})
export const configuredModelSchema = z.object({
  id: z.string(),
  displayName: z.string(),
  contextWindow: z.number().positive(),
  outputLimit: z.number().positive().nullable(),
  thinkingEfforts: z.array(z.string()),
  acceptedExtensions: z.array(z.string()).nullable(),
  fastTier: z.string().nullable(),
})
export const providerSchema = z.object({
  id: z.string(),
  kind: z.enum(['api_key', 'subscription']),
  providerType: z.string(),
  label: z.string(),
  wireApi: z.enum(['responses', 'chat-completions']).nullable(),
  vendorId: z.string().nullable(),
  baseUrl: z.string().nullable(),
  models: z.array(configuredModelSchema).nullable(),
  keyConfigured: z.boolean(),
})
export const providerStateSchema = providerSchema.extend({
  details: providerDetailsSchema.nullable(),
  error: z.string().optional(),
})
const serviceTierSchema = z.object({
  id: z.string(),
  label: z.string(),
  fast: z.boolean().optional(),
})
export const catalogModelSchema = z.object({
  selection: modelSelectionSchema,
  id: z.string(),
  displayName: z.string(),
  contextWindow: z.number().nullable(),
  outputLimit: z.number().nullable(),
  supportsAttachments: z.boolean().nullable(),
  supportsVideo: z.boolean().nullable().optional(),
  acceptedExtensions: z.array(z.string()).nullable().optional(),
  supportedThinkingEfforts: z.array(z.string()).nullable(),
  defaultThinkingEffort: z.string().nullable(),
  canDisableThinking: z.boolean().nullable().optional(),
  serviceTiers: z.array(serviceTierSchema).nullable().optional(),
  defaultServiceTierId: z.string().nullable().optional(),
})
export const catalogProviderSchema = z.object({
  providerId: z.string(),
  displayName: z.string(),
  models: z.array(catalogModelSchema),
  sourceFetchedAt: z.string(),
  stale: z.boolean(),
  warnings: z.array(z.string()),
  auth: healthSchema,
  runtime: healthSchema,
  requiresProcessCapableHost: z.boolean(),
  availability: z.object({
    available: z.boolean(),
    reason: z.string().nullable(),
    message: z.string().nullable(),
  }),
})
export const modelCatalogSchema = z.object({
  providers: z.array(catalogProviderSchema),
})
export const vendorCatalogSchema = z.object({
  subscriptions: z.array(
    z.object({
      providerType: z.string(),
      configured: z.boolean(),
    }),
  ),
  vendors: z.array(
    z.object({
      id: z.string(),
      name: z.string(),
      providerType: z.enum(['openai', 'anthropic', 'google']),
      wireApi: z.enum(['responses', 'chat-completions']).optional(),
      baseUrl: z.string().nullable(),
      doc: z.string().nullable(),
    }),
  ),
})
export const cloudSchema = z.object({
  device: z
    .object({
      id: z.string(),
      name: z.string(),
    })
    .nullable(),
  state: z.enum([
    'unallocated',
    'off',
    'booting',
    'running',
    'saving',
    'resetting',
    'unavailable',
  ]),
  operation: z
    .object({
      id: z.string(),
      phase: z.enum([
        'stopping',
        'saving',
        'rebuilding',
        'booting',
        'ready',
        'failed',
      ]),
      error: z.string().nullable(),
    })
    .nullable(),
  error: z.string().nullable(),
  limits: z.object({
    systemBytes: z.number().nonnegative(),
    homeBytes: z.number().nonnegative(),
  }),
})
export const productStateSchema = z.object({
  user: userSchema,
  mode: z.enum(['shared', 'isolated']),
  preferences: preferencesSchema,
  workspaces: z.array(workspaceSchema),
  devices: z.array(deviceSchema),
  providers: z.array(providerStateSchema),
  conversations: z.array(conversationSummarySchema),
  cloud: cloudSchema.nullable(),
})
export const directorySchema = z.object({
  path: z.string(),
  home: z.string().nullable(),
  entries: z.array(
    z.object({
      name: z.string(),
      isDirectory: z.boolean(),
      size: z.number().nonnegative().nullable(),
      modifiedAt: z.string().nullable(),
    }),
  ),
})
export const fieldResultSchema = z.object({
  field: z.string(),
  status: z.enum(['applied', 'failed']),
  code: z.string().optional(),
  message: z.string().optional(),
})
export const conversationUpdateSchema = z.object({
  conversation: conversationRecordSchema.nullable(),
  results: z.array(fieldResultSchema),
})
export const conversationBatchSchema = z.object({
  results: z.array(
    z.object({
      id: z.string(),
      conversation: conversationRecordSchema.nullable().optional(),
      results: z.array(fieldResultSchema).optional(),
      code: z.string().optional(),
      message: z.string().optional(),
    }),
  ),
})

export type ProductState = z.infer<typeof productStateSchema>
export type BackendConversation = z.infer<typeof conversationSummarySchema>
export type BackendProvider = z.infer<typeof providerStateSchema>
export type CatalogProvider = z.infer<typeof catalogProviderSchema>
export type CatalogModel = z.infer<typeof catalogModelSchema>
export type ConfiguredModel = z.infer<typeof configuredModelSchema>
export type VendorCatalog = z.infer<typeof vendorCatalogSchema>
