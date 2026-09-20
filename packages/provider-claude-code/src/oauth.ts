/**
 * Where a resolved token came from. Callers that inject the token into a
 * spawned CLI must skip `keychain`: that token is the CLI's own short-lived
 * credential, and injecting it via CLAUDE_CODE_OAUTH_TOKEN disables the CLI's
 * refresh flow — the run starts 401ing as soon as the token expires. Owned
 * sources (static/file/env) are the caller's responsibility and inject as-is.
 */
export type ClaudeCodeOAuthSource = 'static' | 'file' | 'env' | 'keychain'

export interface ClaudeCodeOAuthAccess {
  accessToken: string
  source: ClaudeCodeOAuthSource
  subscriptionType?: string | null
  rateLimitTier?: string | null
}

/**
 * The token to inject into a spawned CLI as CLAUDE_CODE_OAUTH_TOKEN, or null
 * to let the CLI authenticate itself. Keychain-sourced tokens are never
 * injected — see {@link ClaudeCodeOAuthSource}.
 */
export function injectableCliToken(access: ClaudeCodeOAuthAccess): string | null {
  return access.source === 'keychain' ? null : access.accessToken
}
