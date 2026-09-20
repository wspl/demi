import { join, resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { contentDigest, nativePackageSchema } from '@demicodes/command-protocol'
import { collectReleaseFiles, selectedTargets, publishReleaseDirectory } from './release-files'
import { z } from 'zod'
import { NATIVE_PACKAGE_CRATES, readPackageInfo } from './package-info'

const { values } = parseArgs({ options: {
  package: { type: 'string' },
  artifacts: { type: 'string' },
  output: { type: 'string' },
  target: { type: 'string', multiple: true },
} })
if (!values.package || !values.artifacts || !values.output)
  throw new Error(`Usage: release-package.ts --package <${NATIVE_PACKAGE_CRATES.join('|')}> --artifacts <cargo-target-directory> --output <release-directory> [--target <triple>]...`)
const crate = z.enum(NATIVE_PACKAGE_CRATES).parse(values.package)
const info = await readPackageInfo(crate)
const output = resolve(values.output)
const { files, targets } = await collectReleaseFiles(resolve(values.artifacts), crate, selectedTargets(values.target))
const descriptor = nativePackageSchema.parse({ ...info, targets })
const hash = await contentDigest(descriptor)
const json = `${JSON.stringify(descriptor, null, 2)}\n`
await publishReleaseDirectory(output, 'descriptor.json', json, files)
console.log(`Native package ${descriptor.id}@${descriptor.version}: ${hash}`)
console.log(join(output, 'descriptor.json'))
