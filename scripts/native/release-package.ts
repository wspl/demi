import { join, resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { contentDigest, nativePackageSchema } from '@demicodes/command-protocol'
import { collectReleaseFiles, selectedTargets, publishReleaseDirectory } from './release-files'
import { readDemiPackageInfo } from './package-info'

const info = await readDemiPackageInfo()

const { values } = parseArgs({ options: {
  artifacts: { type: 'string' },
  output: { type: 'string' },
  target: { type: 'string', multiple: true },
} })
if (!values.artifacts || !values.output)
  throw new Error('Usage: release-package.ts --artifacts <cargo-target-directory> --output <release-directory> [--target <triple>]...')
const output = resolve(values.output)
const { files, targets } = await collectReleaseFiles(resolve(values.artifacts), 'demi-commands', selectedTargets(values.target))
const descriptor = nativePackageSchema.parse({ ...info, targets })
const hash = await contentDigest(descriptor)
const json = `${JSON.stringify(descriptor, null, 2)}\n`
await publishReleaseDirectory(output, 'descriptor.json', json, files)
console.log(`Native package ${descriptor.id}@${descriptor.version}: ${hash}`)
console.log(join(output, 'descriptor.json'))
