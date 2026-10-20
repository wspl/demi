// The model catalog as the components read it. Decoupled from the backend's
// catalog so the component library stays portable: hosts map their own
// catalogs onto these DTOs.

export interface ProviderInfo {
  id: string
  label: string
  isAvailable: boolean
  /**
   * Why an incomplete entry's models cannot be chosen yet, in the backend's
   * words: the model menu lists them dimmed with it as the reason.
   */
  setupReason?: string
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
   * window offers it; null uses the full window (`packages/web-ui/src/agent/context-limit.ts`).
   */
  contextLimit: number | null
  acceptedExtensions: string[] | null
  reasoning: ModelReasoning | null
  /** Provider-advertised speed tiers. Fast Mode is the tier flagged `fast`; models without one have no Fast switch. */
  serviceTiers: ModelServiceTier[] | null
}

// The session's events and the one transcript patch applier, which a host
// reads through here: it reaches `@demicodes/conversation-client` only through this
// package.
export {
  EMPTY_TRANSCRIPT,
  heldTranscript,
  addPage,
  addWindow,
  applyTranscriptPatches,
  heldBlocks,
  heldEdge,
  latestWindow,
  resetTranscript,
  windowEnd,
  windowOf,
  withWholeBlock,
  type ClientSessionEvent,
  type ConversationClient,
  type Held,
  type HeldBlock,
  type HeldTranscript,
  type TranscriptWindow,
} from '@demicodes/conversation-client'
