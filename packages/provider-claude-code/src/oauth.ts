import type { ClaudeCodeAuthStore } from './auth'
import { FileClaudeCodeAuthStore } from './auth'

export interface ClaudeCodeOAuthAccess {
  accessToken: string
  subscriptionType?: string | null
  rateLimitTier?: string | null
}

/**
 * Resolve Claude Code consumer OAuth access for quota APIs.
 * Order: options env CLAUDE_CODE_OAUTH_TOKEN, then macOS Keychain "Claude Code-credentials".
 * Prefer injecting {@link ClaudeCodeAuthStore} when multi-credential is enabled.
 */
export async function resolveClaudeCodeOAuthAccess(): Promise<ClaudeCodeOAuthAccess | null> {
  try {
    return await new FileClaudeCodeAuthStore().resolveAccess()
  } catch {
    return null
  }
}

export async function resolveAccessFromStore(store: ClaudeCodeAuthStore): Promise<ClaudeCodeOAuthAccess | null> {
  try {
    return await store.resolveAccess()
  } catch {
    return null
  }
}
