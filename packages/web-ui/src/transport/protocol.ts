import type { ProviderSelection } from '@demicodes/agent/client'

// Control-plane protocol. Decoupled from @demicodes/provider so the component library stays
// portable: hosts map their own catalogs onto these DTOs.

export interface ProviderInfo {
  id: string
  label: string
  isAvailable: boolean
}

export interface ModelReasoning {
  efforts: string[]
  defaultEffort: string | null
  /** Whether thinking can be turned off entirely. When false, the UI offers only effort levels and
   *  no "No reasoning" option (e.g. Claude Code, which can level thinking but never disable it). */
  canDisable: boolean
}

export interface ModelServiceTier {
  id: string
  label: string
  /** The provider's Fast Mode tier; the Fast switch writes this id. */
  fast: boolean
}

export interface ModelInfo {
  id: string
  name: string
  contextWindow: number | null
  inputLimit: number | null
  acceptedExtensions: string[]
  reasoning: ModelReasoning | null
  /** Provider-advertised speed tiers. Fast Mode is the tier flagged `fast`; models without one have no Fast switch. */
  serviceTiers: ModelServiceTier[] | null
}

export interface PrepareSessionParams {
  providerId: string
  modelId: string
  thinkingEffort?: string | null
  serviceTierId?: string | null
}

export interface WorkspaceInfo {
  cwd: string
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

export type ControlResponse =
  | { id: number; ok: true; result: unknown }
  | { id: number; ok: false; error: string }
