import { writeFile } from 'node:fs/promises'
import { z } from 'zod'
import { nativePackageSchema } from '@demicodes/command-protocol'
import { RustZodTypes } from '../../../scripts/rust-zod'
import { manifestSchema, manifestNodeSchema } from '../src/manifest/schema'

const generator = new RustZodTypes(new Map([[nativePackageSchema, 'demi_command_protocol::PackageDescriptor']]))
generator.type(manifestNodeSchema, 'Node')
generator.type(manifestSchema, 'Manifest')
await writeFile(new URL('../rust/generated.rs', import.meta.url), `// Generated from the TypeScript manifest contract.\n${[...generator.declarations.values()].join('\n\n')}\n`)
await writeFile(new URL('../rust/manifest.schema.json', import.meta.url), `${JSON.stringify(z.toJSONSchema(manifestSchema), null, 2)}\n`)
