import type { ProviderSelection } from '@demi/agent/client'

// Control-plane protocol. Decoupled from @demi/provider so the component library stays
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

export interface ModelInfo {
  id: string
  name: string
  contextWindow: number | null
  inputLimit: number | null
  acceptedExtensions: string[]
  reasoning: ModelReasoning | null
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
