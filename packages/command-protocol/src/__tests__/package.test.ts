import { describe, expect, test } from 'bun:test'
import { canonicalJson, contentDigest, nativePackageSchema } from '../index'
import fixture from '../../tests/fixtures/package.json'

describe('native package contract', () => {
  test('uses the same canonical descriptor identity as the Rust fixture', async () => {
    expect(await contentDigest(nativePackageSchema.parse(fixture.descriptor))).toBe(fixture.digest)
    expect(canonicalJson({ z: 1, a: { b: 2, a: 3 } })).toBe('{"a":{"a":3,"b":2},"z":1}')
  })
  test('describes the targets a release carries, and no others', () => {
    const { 'aarch64-apple-darwin': _removed, ...targets } = fixture.descriptor.targets
    expect(nativePackageSchema.safeParse({ ...fixture.descriptor, targets }).success).toBe(true)
    const unknown = { ...targets, 'riscv64-unknown-linux-musl': _removed }
    expect(nativePackageSchema.safeParse({ ...fixture.descriptor, targets: unknown }).success).toBe(false)
  })
  test('rejects repeated operations and corrupt JSON strings', () => {
    expect(nativePackageSchema.safeParse({ ...fixture.descriptor, operations: ['x', 'x'] }).success).toBe(false)
    expect(() => canonicalJson('\ud800')).toThrow('valid Unicode')
    expect(() => canonicalJson({ '\ud800': 1 })).toThrow('valid Unicode')
  })
})
