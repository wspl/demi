import { writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { z } from 'zod'
import { generateRustValues } from '../../runner-protocol/scripts/generate-rust'
import { NATIVE_TARGETS, nativePackageSchema } from '../src/index'

const schema = z.toJSONSchema(nativePackageSchema)
// JSON Schema carries uniqueness across language boundaries as well.
schema.properties!.operations!.uniqueItems = true
await writeFile(resolve(import.meta.dir, '../src/package.schema.json'), `${JSON.stringify(schema, null, 2)}\n`)
await writeFile(resolve(import.meta.dir, '../src/package_generated.rs'),
  `${generateRustValues({ PackageDescriptor: nativePackageSchema })}\npub const TARGETS: &[&str] = &${JSON.stringify(NATIVE_TARGETS)};\n`)
