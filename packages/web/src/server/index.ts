import process from 'node:process'
import type { Provider } from '@demi/provider'
import { createClaudeCodeProvider, resolveWireLogDir } from '@demi/provider-claude-code'
import { createCodexProvider } from '@demi/provider-codex'
import { parseServerOptions } from './server-options'
import { startWebServer } from './serve'

const options = parseServerOptions(process.argv.slice(2))
const claudeCodeProvider = createClaudeCodeProvider({ claudePath: options.claudePath })
const codexProvider = createCodexProvider({
  codexHome: options.codexHome,
  baseUrl: options.baseUrl,
  transport: options.transport,
})
const providers: Provider[] =
  options.provider === 'codex'
    ? [codexProvider, claudeCodeProvider]
    : [claudeCodeProvider, codexProvider]

const handle = startWebServer(providers, options)
process.stdout.write(`demi web listening on ${handle.url} — cwd ${options.cwd}, provider ${options.provider}\n`)
const wireLogDir = resolveWireLogDir()
process.stdout.write(`claude wire log: ${wireLogDir ?? '(disabled)'}\n`)
