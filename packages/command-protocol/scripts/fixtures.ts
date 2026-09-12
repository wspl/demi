import { writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { contentDigest, NATIVE_TARGETS, nativePackageSchema } from '../src/index'

const descriptor = nativePackageSchema.parse({
  id: 'demicodes.fixture',
  version: 'test-α-😀',
  protocolVersion: 1,
  operations: ['file.read', 'fixture.echo'],
  targets: Object.fromEntries(NATIVE_TARGETS.map((target, index) => [target, {
    sha256: index.toString(16).repeat(64),
    size: 12345 + index,
  }])),
})
await writeFile(resolve(import.meta.dir, '../tests/fixtures/package.json'),
  `${JSON.stringify({ descriptor, digest: await contentDigest(descriptor) }, null, 2)}\n`)
