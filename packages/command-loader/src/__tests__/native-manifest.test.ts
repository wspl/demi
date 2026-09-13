import { expect, test } from 'bun:test'
import { nativePackageSchema } from '@demicodes/command-protocol'
import type { Command } from '@demicodes/shell'
import fixture from '../../../command-protocol/tests/fixtures/package.json'
import { buildManifest, verifyManifest } from '../index'

const descriptor = nativePackageSchema.parse(fixture.descriptor)
const roots: Command[] = [{
  name: 'native',
  summary: 'Native declaration.',
  kind: 'native',
  binding: { package: descriptor.id, operation: descriptor.operations[0]! },
}]

test('native manifests bind a complete exact package and verify after JSON transport', async () => {
  const manifest = await buildManifest(roots, { packages: [descriptor] })
  expect(await verifyManifest(JSON.parse(JSON.stringify(manifest)))).toEqual(manifest)
  expect(Object.keys(manifest.packages)).toEqual([fixture.digest])
  expect(manifest.roots.native!.tree).toMatchObject({
    kind: 'native',
    binding: { package: descriptor.id, descriptorHash: fixture.digest },
  })
  expect(manifest).not.toHaveProperty('modules')
  const changed = structuredClone(manifest)
  changed.roots.native!.tree.summary = 'Changed declaration.'
  await expect(verifyManifest(changed)).rejects.toThrow('hash mismatch')
})

test('native catalog rejects duplicate ids, missing bindings and missing operations', async () => {
  await expect(buildManifest(roots, { packages: [descriptor, descriptor] })).rejects.toThrow('Duplicate native package')
  await expect(buildManifest(roots, { packages: [] })).rejects.toThrow('not configured')
  const missing: Command[] = [{ ...roots[0]!, name: 'unknown', kind: 'native', binding: { package: descriptor.id, operation: 'missing' } }]
  await expect(buildManifest(missing, { packages: [descriptor] })).rejects.toThrow('Unresolved native binding')
})
