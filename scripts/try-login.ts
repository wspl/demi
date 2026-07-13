// Manual end-to-end trial for the native credential login flows.
//
//   bun scripts/try-login.ts codex
//   bun scripts/try-login.ts grok-build
//   bun scripts/try-login.ts claude-code
//
// Uses a throwaway state dir by default (printed at the end) so trials never touch a real
// pool; pass --state-dir <dir> to import into a specific pool. Complete the login from any
// browser on any device; claude-code additionally asks you to paste the code shown by the
// vendor page after approval.
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { ProviderCredentialLoginPending, ProviderCredentials } from '@demicodes/provider'
import { PoolAwareCodexAuthStore, createCodexCredentials, openCodexCredentialPool } from '@demicodes/provider-codex'
import { PoolAwareGrokAuthStore, createGrokBuildCredentials, openGrokCredentialPool } from '@demicodes/provider-grok-build'
import { PoolAwareClaudeCodeAuthStore, createClaudeCodeCredentials, openClaudeCodeCredentialPool } from '@demicodes/provider-claude-code'

const providerId = process.argv[2]
const stateDirFlag = process.argv.indexOf('--state-dir')
const stateDir = stateDirFlag > 0 ? process.argv[stateDirFlag + 1]! : mkdtempSync(join(tmpdir(), 'demi-login-trial-'))

function buildCredentials(): ProviderCredentials {
  if (providerId === 'codex') {
    const pool = openCodexCredentialPool({ stateDir })
    return createCodexCredentials(pool, new PoolAwareCodexAuthStore(pool), {})
  }
  if (providerId === 'grok-build') {
    const pool = openGrokCredentialPool({ stateDir })
    return createGrokBuildCredentials(pool, new PoolAwareGrokAuthStore(pool), {})
  }
  if (providerId === 'claude-code') {
    const pool = openClaudeCodeCredentialPool({ stateDir })
    return createClaudeCodeCredentials(pool, new PoolAwareClaudeCodeAuthStore(pool), {})
  }
  console.error('Usage: bun scripts/try-login.ts <codex|grok-build|claude-code> [--state-dir <dir>]')
  process.exit(1)
}

async function readLine(prompt: string): Promise<string> {
  process.stdout.write(prompt)
  for await (const line of console) return line.trim()
  return ''
}

const credentials = buildCredentials()
const result = await credentials.beginLogin!({
  onPending: (pending: ProviderCredentialLoginPending) => {
    console.log('\n=== ACTION REQUIRED (any browser, any device) ===')
    console.log('Open:', pending.verificationUrl)
    if (pending.userCode) console.log('Enter code:', pending.userCode)
    if (pending.expiresAt) console.log('Expires at:', pending.expiresAt)
    if (pending.requiresCodeInput) console.log('After approving, the page shows a code — paste it back here.')
    console.log('================================================\n')
  },
  promptForCode: () => readLine('Paste the code from the vendor page: '),
})

console.log('\nresult:', JSON.stringify(result))
if (result.status === 'completed') {
  console.log('pool entries:', JSON.stringify(await credentials.list(), null, 2))
  console.log('auth status:', JSON.stringify(await credentials.getActive()))
}
console.log('state dir:', stateDir)
process.exit(result.status === 'completed' ? 0 : 1)
