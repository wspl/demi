import process from 'node:process'
import { ProviderRegistry } from '@demi/provider'
import { parseServerOptions } from './server-options'
import { startWebServer } from './serve'
import { createStubProviderDefinition } from './stub-provider'

const options = parseServerOptions(process.argv.slice(2))
const registry = new ProviderRegistry()
registry.register(createStubProviderDefinition())

const handle = startWebServer(registry, options)
process.stdout.write(`demi web (stub provider) listening on ${handle.url} — cwd ${options.cwd}\n`)
