import process from 'node:process'
import { parseServerOptions } from './server-options'
import { startWebServer } from './serve'
import { createStubProvider } from './stub-provider'

const options = parseServerOptions(process.argv.slice(2))

const handle = startWebServer([createStubProvider()], options)
process.stdout.write(`demi web (stub provider) listening on ${handle.url} — cwd ${options.cwd}\n`)
