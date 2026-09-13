import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'
import { contentDigest, nativePackageSchema } from '@demicodes/command-protocol'
import { collectReleaseFiles, publishReleaseDirectory, writeReleasePointer } from '../../../scripts/native/release-files'
import info from '../package-info.json'

const { values } = parseArgs({ options: {
  artifacts: { type: 'string' },
  output: { type: 'string' },
} })
if (!values.artifacts || !values.output)
  throw new Error('Usage: release.ts --artifacts <cargo-target-directory> --output <release-directory>')
const output = resolve(values.output)
const { files, targets } = await collectReleaseFiles(resolve(values.artifacts), 'demi-commands')
const descriptor = nativePackageSchema.parse({ ...info, targets })
const hash = await contentDigest(descriptor)
const json = `${JSON.stringify(descriptor, null, 2)}\n`
await publishReleaseDirectory(output, 'descriptor.json', json, files)
await writeReleasePointer(fileURLToPath(new URL('../src/release.json', import.meta.url)), json)
console.log(`Native package ${descriptor.id}@${descriptor.version}: ${hash}`)
console.log(join(output, 'descriptor.json'))
