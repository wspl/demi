import process from 'node:process'
import { ProviderRegistry } from '@demi/provider'
import { createClaudeCodeProviderDefinition, resolveWireLogDir } from '@demi/provider-claude-code'
import { createCodexProviderDefinition } from '@demi/provider-codex'
import { parseServerOptions } from './server-options'
import { startWebServer } from './serve'

const options = parseServerOptions(process.argv.slice(2))
const registry = new ProviderRegistry()
registry.register(createClaudeCodeProviderDefinition())
registry.register(createCodexProviderDefinition())

const handle = startWebServer(registry, options)
process.stdout.write(`demi web listening on ${handle.url} — cwd ${options.cwd}, provider ${options.provider}\n`)
const wireLogDir = resolveWireLogDir()
process.stdout.write(`claude wire log: ${wireLogDir ?? '(disabled)'}\n`)
