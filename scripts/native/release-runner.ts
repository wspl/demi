import { createHash } from 'node:crypto'
import { join, resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { canonicalJson, NATIVE_PROTOCOL_VERSION } from '@demicodes/command-protocol'
import { RUNNER_PROTOCOL_VERSION } from '@demicodes/runner-protocol'
import { runnerReleaseSchema } from '@demicodes/runner-protocol/release'
import { collectReleaseFiles, selectedTargets, publishReleaseDirectory, writeReleasePointer } from './release-files'

const { values } = parseArgs({ options: {
  artifacts: { type: 'string' },
  output: { type: 'string' },
  target: { type: 'string', multiple: true },
} })
if (!values.artifacts || !values.output)
  throw new Error('Usage: release-runner.ts --artifacts <cargo-target-directory> --output <release-directory> [--target <triple>]...')
const output = resolve(values.output)
const { files, targets } = await collectReleaseFiles(resolve(values.artifacts), 'demi-runner', selectedTargets(values.target))
const contents = { wire: RUNNER_PROTOCOL_VERSION, commandProtocol: NATIVE_PROTOCOL_VERSION, targets }
const release = createHash('sha256').update(canonicalJson(contents)).digest('hex')
const manifest = runnerReleaseSchema.parse({ release, ...contents })
const json = `${JSON.stringify(manifest, null, 2)}\n`
const directory = join(output, release)
await publishReleaseDirectory(directory, 'manifest.json', json, files)
await writeReleasePointer(join(output, 'manifest.json'), json)
console.log(`Runner release: ${directory}`)
