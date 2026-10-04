// The model catalog as the components read it. Decoupled from the backend's
// catalog so the component library stays portable: hosts map their own
// catalogs onto these DTOs.

export interface ProviderInfo {
  id: string
  label: string
  isAvailable: boolean
}

export interface ModelReasoning {
  /** The efforts the model lists; settings that name none hold the first. */
  efforts: string[]
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
  /**
   * The limit the user set on the window Demi uses for this model, while its
   * window offers it; null uses the full window (`models.md` § Context limit).
   */
  contextLimit: number | null
  inputLimit: number | null
  acceptedExtensions: string[] | null
  reasoning: ModelReasoning | null
  /** Provider-advertised speed tiers. Fast Mode is the tier flagged `fast`; models without one have no Fast switch. */
  serviceTiers: ModelServiceTier[] | null
}

// The session's events and the one transcript patch applier, which a host
// reads through here: it reaches `@demicodes/conversation-client` only through this
// package.
export { applyTranscriptPatches, type ClientSessionEvent } from '@demicodes/conversation-client'
