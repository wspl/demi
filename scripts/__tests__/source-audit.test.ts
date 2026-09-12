import { expect, test } from 'bun:test'
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { functionImplementations, listProductionSources, moduleSpecifiers, parseSourcePrograms, unvalidatedJsonAssertions } from '../source-audit'

function implementations(file: string, source: string) {
  return parseSourcePrograms(file, source).flatMap(({ program }) => functionImplementations(program))
}

test('production discovery includes JS, TS and Vue but excludes tests, fixtures and declarations', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-source-audit-'))
  const files = ['index.ts', 'client.js', 'view.vue', 'nested/value.mts', 'component.tsx',
    'index.test.ts', 'client.spec.js', 'globals.d.ts', 'testing.ts',
    '__tests__/case.ts', 'fixtures/view.vue', 'testing/host.ts']
  try {
    for (const file of files) {
      const path = join(root, file)
      await mkdir(join(path, '..'), { recursive: true })
      await writeFile(path, '')
    }
    expect((await listProductionSources(root)).map(file => file.slice(root.length + 1)))
      .toEqual(['client.js', 'component.tsx', 'index.ts', 'nested/value.mts', 'view.vue'])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('Vue imports come from both actual script blocks and external scripts, never template text', () => {
  const source = `<template><p>import example from '@demicodes/backend'</p></template>
<script lang="ts">import type { User } from '@demicodes/product-contracts'; export default {}</script>
<script setup lang="ts">const view = () => import('@demicodes/web-ui/agent');</script>`
  expect(parseSourcePrograms('Example.vue', source).flatMap(({ program }) => moduleSpecifiers(program)))
    .toEqual(['@demicodes/product-contracts', '@demicodes/web-ui/agent'])
  expect(parseSourcePrograms('External.vue', '<script src="./external.ts"></script>')
    .flatMap(({ program }) => moduleSpecifiers(program))).toEqual(['./external.ts'])
  expect(() => parseSourcePrograms('Bad.vue', '<script setup lang="ts">const =</script>'))
    .toThrow('Unable to parse')
})

test('runtime AST comparisons find renamed function and arrow copies, including Vue script setup', () => {
  const original = implementations('helper.ts',
    'export function numberOrNull(value: unknown): number | null { return typeof value === "number" && Number.isFinite(value) ? value : null }')[0]!
  for (const [file, source] of [
    ['copy.js', 'const renamed = input => typeof input === "number" && Number.isFinite(input) ? input : null'],
    ['copy.ts', 'const numberOrNull = function (other: unknown) { return typeof other === "number" && Number.isFinite(other) ? other : null }'],
    ['copy.vue', '<script setup lang="ts">const copied = (n: unknown) => typeof n === "number" && Number.isFinite(n) ? n : null</script>'],
  ]) {
    expect(implementations(file!, source!)[0]?.fingerprint).toBe(original.fingerprint)
  }
  const different = implementations('different.ts',
    'function numberOrNull(value: unknown) { return typeof value === "number" && Number.isFinite(value) ? value : 0 }')[0]!
  expect(different.fingerprint).not.toBe(original.fingerprint)
  expect(parseSourcePrograms('import.ts', 'import { numberOrNull } from "@demicodes/utils"')
    .flatMap(({ program }) => functionImplementations(program))).toEqual([])
})

test('AST comparison preserves property names and skips scope-sensitive function forms', () => {
  const property = implementations('a.ts', 'const first = value => value.value')[0]!
  const renamed = implementations('b.ts', 'const second = item => item.value')[0]!
  const otherProperty = implementations('c.ts', 'const third = item => item.item')[0]!
  expect(property.fingerprint).toBe(renamed.fingerprint)
  expect(property.fingerprint).not.toBe(otherProperty.fingerprint)
  expect(implementations('scope.ts', 'const first = () => this.value')).toEqual([])
  expect(implementations('default.ts', 'const first = (value = 3) => value')).toEqual([])
})


test('direct JSON assertion checks cover async arrows and Vue without banning narrowing or schemas', () => {
  for (const [file, source] of [
    ['parse.ts', 'const parse = (text: string) => JSON.parse(text) as Value'],
    ['read.ts', 'const read = async () => (await response.json()) as unknown as Value'],
    ['view.vue', '<script setup lang="ts">const value: Value = JSON.parse(raw)</script>'],
  ]) {
    expect(parseSourcePrograms(file!, source!).flatMap(({ program }) => unvalidatedJsonAssertions(program)).length)
      .toBeGreaterThan(0)
  }
  const valid = `const raw: unknown = JSON.parse(text);
    const value = schema.parse(raw);
    const narrowed = typeof input === 'string' ? input : null;
    const typed = responseSchema.parse(await response.json()) as Value;`
  expect(parseSourcePrograms('valid.ts', valid).flatMap(({ program }) => unvalidatedJsonAssertions(program))).toEqual([])
})
