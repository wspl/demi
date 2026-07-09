import { isRecord, nonEmptyString } from '@demicodes/utils'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import process from 'node:process'

const execFileAsync = promisify(execFile)

export interface ClaudeCodeOAuthAccess {
  accessToken: string
  subscriptionType?: string | null
  rateLimitTier?: string | null
}

/**
 * Resolve Claude Code consumer OAuth access for quota APIs.
 * Order: options env CLAUDE_CODE_OAUTH_TOKEN, then macOS Keychain "Claude Code-credentials".
 */
export async function resolveClaudeCodeOAuthAccess(): Promise<ClaudeCodeOAuthAccess | null> {
  const fromEnv = nonEmptyString(process.env.CLAUDE_CODE_OAUTH_TOKEN)
  if (fromEnv) return { accessToken: fromEnv }

  if (process.platform === 'darwin') {
    try {
      const { stdout } = await execFileAsync(
        'security',
        ['find-generic-password', '-s', 'Claude Code-credentials', '-w'],
        { encoding: 'utf8', timeout: 5_000 },
      )
      const parsed = JSON.parse(stdout.trim()) as unknown
      if (!isRecord(parsed)) return null
      const oauth = isRecord(parsed.claudeAiOauth) ? parsed.claudeAiOauth : null
      if (!oauth) return null
      const accessToken = nonEmptyString(oauth.accessToken)
      if (!accessToken) return null
      return {
        accessToken,
        subscriptionType: nonEmptyString(oauth.subscriptionType) ?? null,
        rateLimitTier: nonEmptyString(oauth.rateLimitTier) ?? null,
      }
    } catch {
      return null
    }
  }

  return null
}
