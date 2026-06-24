import process from 'node:process'
import { resolveWireLogDir } from '@demi/provider-claude-code'
import { createWebProviders } from './providers'
import { parseServerOptions } from './server-options'
import { startWebServer } from './serve'

const options = parseServerOptions(process.argv.slice(2))
const providers = createWebProviders(options)

const handle = startWebServer(providers, options)
process.stdout.write(`demi web listening on ${handle.url} — cwd ${options.cwd}, provider ${options.provider}\n`)
const wireLogDir = resolveWireLogDir()
process.stdout.write(`claude wire log: ${wireLogDir ?? '(disabled)'}\n`)
