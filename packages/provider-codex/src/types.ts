/** How the provider reaches the Responses backend. */
export const CODEX_TRANSPORT_MODES = ['auto', 'sse', 'websocket'] as const

export type CodexTransportMode = (typeof CODEX_TRANSPORT_MODES)[number]
