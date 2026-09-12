import { z } from 'zod'
import { modelSelectionSchema } from '@demicodes/agent/client'
import { userSchema, instanceModeSchema } from './auth'
import { preferencesSchema } from './preferences'
import { configuredModelSchema } from './models'
import { targetSchema } from './conversations'

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

const optionalMessage = { message: z.string().optional() }
const unknownHealthSchema = z.object({
  status: z.literal('unknown'),
  ...optionalMessage,
})
const errorHealthSchema = z.object({
  status: z.literal('error'),
  message: z.string(),
})
export const providerAuthSchema = z.discriminatedUnion('status', [
  unknownHealthSchema,
  z.object({
    status: z.literal('authenticated'),
    accountLabel: z.string().optional(),
    ...optionalMessage,
  }),
  z.object({ status: z.literal('unauthenticated'), ...optionalMessage }),
  errorHealthSchema,
])
export const providerRuntimeSchema = z.discriminatedUnion('status', [
  unknownHealthSchema,
  z.object({ status: z.literal('ready'), ...optionalMessage }),
  z.object({ status: z.literal('unavailable'), message: z.string() }),
  errorHealthSchema,
])
const credentialSchema = z.object({
  id: z.string(),
  label: z.string(),
  detail: z.string().nullable().optional(),
})
const activeCredentialSchema = z.object({
  credentialId: z.string().nullable(),
  status: providerAuthSchema,
})
export const quotaSchema = z.object({
  providerId: z.string().optional(),
  source: z.enum(['probe', 'observation', 'cache']).optional(),
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
  auth: providerAuthSchema,
  runtime: providerRuntimeSchema,
  accounts: z.array(credentialSchema),
  active: activeCredentialSchema.nullable(),
  credentials: z
    .discriminatedUnion('mode', [
      z.object({ mode: z.literal('none') }),
      z.object({
        mode: z.literal('supported'),
        canBeginLogin: z.boolean().optional(),
        canImportDefault: z.boolean().optional(),
        canAdd: z.boolean().optional(),
        multi: z.boolean().optional(),
      }),
    ])
    .optional(),
  quota: quotaSchema.nullable(),
  quotaCapability: z.object({
    mode: z.enum(['none', 'supported']),
    canProbe: z.boolean().optional(),
    probeCost: z.enum(['free', 'minimal_request']).optional(),
  }),
  requiresProcessCapableHost: z.boolean(),
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
  createdAt: z.iso.datetime({ offset: true }).optional(),
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
  auth: providerAuthSchema,
  runtime: providerRuntimeSchema,
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
  mode: instanceModeSchema,
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
export type VendorCatalog = z.infer<typeof vendorCatalogSchema>
export type PublicProvider = z.infer<typeof providerSchema>
export type ProviderDetails = z.infer<typeof providerDetailsSchema>
export type PublicQuota = z.infer<typeof quotaSchema>
export type ModelCatalog = z.infer<typeof modelCatalogSchema>
