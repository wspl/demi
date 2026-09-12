import { expect, test } from 'bun:test'
import {
  packageManifestSchema,
  registryVersionsSchema,
  workspaceManifestSchema,
} from '../release-contracts'

test('release manifests preserve publication metadata and reject malformed consumed fields', () => {
  const manifest = {
    name: '@demicodes/probe',
    version: '1.0.0',
    exports: { '.': { import: ['./dist/main.js', null] } },
    dependencies: { zod: '^4.0.0' },
    license: 'Apache-2.0',
    files: ['dist'],
  }
  expect(packageManifestSchema.parse(manifest)).toEqual(manifest)
  for (const value of [
    null,
    {},
    { ...manifest, private: 'false' },
    { ...manifest, version: 1 },
    { ...manifest, exports: { '.': 42 } },
    { ...manifest, dependencies: { zod: 4 } },
  ]) {
    expect(packageManifestSchema.safeParse(value).success).toBe(false)
  }
  expect(registryVersionsSchema.safeParse({}).success).toBe(false)
  expect(registryVersionsSchema.safeParse({ versions: [] }).success).toBe(false)
  expect(
    Object.keys(
      registryVersionsSchema.parse({ versions: { '1.0.0': {} } }).versions,
    ),
  ).toEqual(['1.0.0'])
  expect(
    workspaceManifestSchema.safeParse({ workspaces: [null] }).success,
  ).toBe(false)
})
