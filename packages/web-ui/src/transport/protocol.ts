import { z } from 'zod'
import { providerSelectionSchema } from '@demicodes/agent/client'
import type {
  ProviderSelection,
  DisplayedBlock,
  AgentClient as CoreAgentClient,
  ServerFrame as AgentServerFrame,
  ClientSessionEvent as AgentSessionEvent,
} from '@demicodes/agent/client'

export type AgentClient = CoreAgentClient<DisplayedBlock>
export type ServerFrame = AgentServerFrame<DisplayedBlock>
export type ClientSessionEvent = AgentSessionEvent<DisplayedBlock>

// Control-plane protocol. Decoupled from @demicodes/provider so the component library stays
// portable: hosts map their own catalogs onto these DTOs.

export const providerInfoSchema = z.object({
  id: z.string().min(1),
  label: z.string(),
  isAvailable: z.boolean(),
})
export type ProviderInfo = z.infer<typeof providerInfoSchema>

export const modelReasoningSchema = z.object({
  efforts: z.array(z.string()),
  defaultEffort: z.string().nullable(),
  canDisable: z.boolean(),
})
export type ModelReasoning = z.infer<typeof modelReasoningSchema>

export const modelServiceTierSchema = z.object({
  id: z.string().min(1),
  label: z.string(),
  fast: z.boolean(),
})
export type ModelServiceTier = z.infer<typeof modelServiceTierSchema>

export const modelInfoSchema = z.object({
  id: z.string().min(1),
  name: z.string(),
  contextWindow: z.number().int().positive().nullable(),
  inputLimit: z.number().int().positive().nullable(),
  acceptedExtensions: z.array(z.string()).nullable(),
  reasoning: modelReasoningSchema.nullable(),
  serviceTiers: z.array(modelServiceTierSchema).nullable(),
})
export type ModelInfo = z.infer<typeof modelInfoSchema>

export const prepareSessionParamsSchema = z.object({
  providerId: z.string().min(1),
  modelId: z.string().min(1),
  thinkingEffort: z.string().nullable().optional(),
  serviceTierId: z.string().nullable().optional(),
})
export type PrepareSessionParams = z.infer<typeof prepareSessionParamsSchema>
// Empty ids mean the composer has not selected a provider or model yet.
export const modelIntentSchema = prepareSessionParamsSchema.required().extend({
  providerId: z.string(),
  modelId: z.string(),
})

export const workspaceInfoSchema = z.object({ cwd: z.string().min(1) })
export type WorkspaceInfo = z.infer<typeof workspaceInfoSchema>

export const controlResults = {
  listProviders: z.array(providerInfoSchema),
  listModels: z.array(modelInfoSchema),
  prepareSession: providerSelectionSchema,
  defaultWorkspace: workspaceInfoSchema,
}

export interface ControlApi {
  listProviders(): Promise<ProviderInfo[]>
  listModels(params: { providerId: string }): Promise<ModelInfo[]>
  prepareSession(params: PrepareSessionParams): Promise<ProviderSelection>
  defaultWorkspace(): Promise<WorkspaceInfo>
}

export type ControlMethod = keyof ControlApi

export interface ControlRequest {
  id: number
  method: ControlMethod
  params: unknown
}

export const controlResponseSchema = z.discriminatedUnion('ok', [
  z.object({ id: z.number().int().positive(), ok: z.literal(true), result: z.unknown() }),
  z.object({ id: z.number().int().positive(), ok: z.literal(false), error: z.string() }),
])
export type ControlResponse = z.infer<typeof controlResponseSchema>

export {
  displayedBlockSchema,
  displayedUserContentSchema,
  displayedServerFrameSchema,
  usageSchema,
  shellStatusSchema,
  modelSelectionSchema,
  thinkingConfigSchema,
  editRequestSchema,
  editResultSchema,
  transcriptVersionSchema,
} from '@demicodes/agent/client'
export type { ProviderSelection, ClientFrame, DisplayedBlock, DisplayedUserContent, DisplayedToolContent } from '@demicodes/agent/client'

export { applyTranscriptPatches } from '@demicodes/agent/client'
