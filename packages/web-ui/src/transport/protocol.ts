// The model catalog as the components read it. Decoupled from
// @demicodes/provider so the component library stays portable: hosts map their
// own catalogs onto these DTOs.

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
  acceptedExtensions: string[] | null
  reasoning: ModelReasoning | null
  /** Provider-advertised speed tiers. Fast Mode is the tier flagged `fast`; models without one have no Fast switch. */
  serviceTiers: ModelServiceTier[] | null
}

// The agent owns the session contract; an embedder reads it through here so it
// never has to describe a frame, a block or a tool view a second time.
export {
  modelSelectionSchema,
  editRequestSchema,
  blockSchema,
  failuresSchema,
  refSourceSchema,
  shellToolViewSchema,
} from '@demicodes/agent/client'
export type { ProviderSelection, ClientFrame, ClientSessionEvent } from '@demicodes/agent/client'

export { applyTranscriptPatches } from '@demicodes/agent/client'
