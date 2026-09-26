import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { z } from 'zod'
import { msgpackCodec } from '../packages/runner-protocol/src/codec'
import { stringifyPortableJson } from '../packages/utils/src/json'
import { goContractPackages, type GoContractPackage } from './generate-go-contracts'
import { resolveLazy, unionOptions } from './go-zod'

// Records the conformance corpus of the Go contracts
// (internal/contract/<name>/testdata/corpus.json). For every root it builds
// values that exercise each union option and optional field, and mutants
// that break each constraint once; Zod judges each, and the TypeScript codecs
// encode the input and, when Zod accepts it, the parsed value. The Go test
// must reach the same verdict and the same bytes.
//
// bun --conditions=development scripts/record-go-contracts.ts

type Schema = z.core.$ZodType
type Def = z.core.$ZodTypes['_zod']['def']
type Codec = 'json' | 'msgpack'

const ABSENT = Symbol('absent')
const MAX_DEPTH = 6

class TooDeep extends Error {}

function defOf(schema: Schema): Def {
  return (schema as z.core.$ZodTypes)._zod.def
}

function checksOf(schema: Schema): z.core.$ZodCheckDef[] {
  return (defOf(schema).checks ?? []).map(check => (check as z.core.$ZodChecks)._zod.def)
}

/** Sample strings for each regex the contracts use, keyed by its source. */
const PATTERN_SAMPLES: Record<string, string[]> = {
  '^https?:\\/\\/[^\\s]+$': ['https://demi.example/api', 'http://localhost:3271'],
  '^\\S+$': ['token-123', 'déjà-vu'],
  '^[a-f0-9]{64}$': ['a'.repeat(64), '0123456789abcdef'.repeat(4), 'f'.repeat(64)],
  '^[a-z0-9]+(?:[.-][a-z0-9]+)+$': ['demi.commands', 'demi-claude.v2'],
  '^(?:[MTADRC][ MTDAR]| [MTDAR]|\\?\\?|DD|AU|UD|UA|DU|AA|UU)$': ['M ', ' M', '??', 'UU'],
  '^[^\\0]*$': ['/home/demi/work', 'dir with spaces/ü'],
  '^[^\\0=]+$': ['PATH', 'LANG'],
  '^[A-Za-z0-9][A-Za-z0-9_-]*$': ['files', 'git-show', 'x_1'],
  '^t_[A-Za-z0-9_-]{22}$': ['t_abcdefghijklmnopqrstuv', 't_ABCDEFGHIJKLMNOPQRS_-9'],
  '^e_[A-Za-z0-9_-]{22}$': ['e_abcdefghijklmnopqrstuv', 'e_0123456789012345678901'],
  '^\\d+\\.\\d+\\.\\d+\\.\\d+$': ['131.0.6778.85'],
  '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$': ['0f8fad5b-d9cb-469f-a165-70867728950e'],
  '^[^/\\\\\\0]+$': ['report.pdf', 'naïve name.txt'],
  '^[A-Za-z0-9_-]+$': ['gen_1', 'base-2'],
}

const PLAIN_STRINGS = ['text', 'héllo "quoted" \\ back\nslash\t😀 <&> \u0001']

function stringSamples(schema: Schema): string[] {
  const checks = checksOf(schema)
  const def = defOf(schema)
  const regex = checks.find(check => check.check === 'string_format' && (check as z.core.$ZodCheckStringFormatDef).format === 'regex') as z.core.$ZodCheckRegexDef | undefined
  let base: string[]
  if (regex) {
    base = PATTERN_SAMPLES[regex.pattern.source] ?? []
    if (base.length === 0)
      throw new Error(`No samples for pattern ${regex.pattern.source}`)
  } else if ('format' in def && def.format === 'url') {
    base = ['https://example.com/path?q=1#frag', 'http://localhost:8080/']
  } else {
    base = PLAIN_STRINGS
  }
  const min = (checks.find(check => check.check === 'min_length') as z.core.$ZodCheckMinLengthDef | undefined)?.minimum ?? 0
  const max = (checks.find(check => check.check === 'max_length') as z.core.$ZodCheckMaxLengthDef | undefined)?.maximum
  const fitted = base.map(text => {
    let points = [...text]
    if (max !== undefined && points.length > max)
      points = points.slice(0, max)
    while (points.length < min)
      points.push('x')
    return points.join('')
  })
  // Zod counts code points: a string of astral characters at the limit fits.
  if (!regex && max !== undefined && max >= 2 && max <= 64)
    fitted.push('😀'.repeat(max))
  return [...new Set(fitted)]
}

interface Bounds {
  min: number
  max: number
  minInclusive: boolean
  maxInclusive: boolean
}

function numberBounds(schema: Schema): Bounds {
  const bounds: Bounds = { min: -Infinity, max: Infinity, minInclusive: true, maxInclusive: true }
  for (const check of checksOf(schema)) {
    if (check.check === 'greater_than') {
      const constraint = check as z.core.$ZodCheckGreaterThanDef
      bounds.min = Number(constraint.value)
      bounds.minInclusive = constraint.inclusive
    }
    if (check.check === 'less_than') {
      const constraint = check as z.core.$ZodCheckLessThanDef
      bounds.max = Number(constraint.value)
      bounds.maxInclusive = constraint.inclusive
    }
  }
  return bounds
}

function isInt(schema: Schema): boolean {
  return checksOf(schema).some(check => check.check === 'number_format'
    && (check as z.core.$ZodCheckNumberFormatDef).format === 'safeint')
}

function inBounds(value: number, bounds: Bounds): boolean {
  return (bounds.minInclusive ? value >= bounds.min : value > bounds.min)
    && (bounds.maxInclusive ? value <= bounds.max : value < bounds.max)
}

function numberSamples(schema: Schema): number[] {
  const bounds = numberBounds(schema)
  const candidates = isInt(schema)
    ? [0, 1, 127, 200, 70_000, 2 ** 33, -1, -200, -70_000, Number.MAX_SAFE_INTEGER]
    : [0, 1.5, -2.25, 1e21, 1e-7, 123456.789, 2 ** 60]
  const edges = [bounds.min, bounds.max].filter(value => Number.isFinite(value) && inBounds(value, bounds))
  const inside = candidates.filter(value => inBounds(value, bounds))
  const picked = [...new Set([...edges, ...inside.slice(0, 4)])]
  if (picked.length === 0)
    throw new Error('No number sample fits the bounds')
  return picked
}

/** Values the schema should accept, covering each option and optional field. */
function samples(schema: Schema, depth = 0): unknown[] {
  if (depth > MAX_DEPTH)
    throw new TooDeep()
  const def = defOf(schema)
  switch (def.type) {
    case 'string': return stringSamples(schema)
    case 'number': return numberSamples(schema)
    case 'boolean': return [true, false]
    case 'custom': return [new Uint8Array([1, 2, 3]), new Uint8Array(0), new Uint8Array(300).map((_, index) => index % 256)]
    case 'date': return [new Date(1_700_000_000_123), new Date(0), new Date(-1_500), new Date(2 ** 34 * 1000 + 5)]
    case 'unknown': return [null, { a: [1, 'x', true, null, 1.5], b: { c: -3 } }]
    case 'null': return [null]
    case 'literal': return [...def.values]
    case 'enum': return Object.values(def.entries)
    case 'optional':
      if (depth > MAX_DEPTH - 3)
        return [ABSENT]
      return [ABSENT, ...guarded(() => samples(def.innerType, depth + 1))]
    case 'nullable': return [null, ...guarded(() => samples(def.innerType, depth + 1))]
    case 'lazy': return samples(resolveLazy(schema), depth + 1)
    case 'union': {
      const found = unionOptions(schema).flatMap(option => guarded(() => samples(option, depth + 1)))
      if (found.length === 0)
        throw new TooDeep()
      return found
    }
    case 'array': return arraySamples(schema, def, depth)
    case 'record': return recordSamples(def, depth)
    case 'object': return objectSamples(def, depth)
    default: throw new Error(`No samples for ${def.type}`)
  }
}

function guarded(build: () => unknown[]): unknown[] {
  try {
    return build()
  } catch (error) {
    if (error instanceof TooDeep)
      return []
    throw error
  }
}

function lengthBounds(schema: Schema): { min: number, max: number } {
  const checks = checksOf(schema)
  return {
    min: (checks.find(check => check.check === 'min_length') as z.core.$ZodCheckMinLengthDef | undefined)?.minimum ?? 0,
    max: (checks.find(check => check.check === 'max_length') as z.core.$ZodCheckMaxLengthDef | undefined)?.maximum ?? Infinity,
  }
}

function arraySamples(schema: Schema, def: z.core.$ZodArrayDef, depth: number): unknown[] {
  const { min, max } = lengthBounds(schema)
  const items = depth > MAX_DEPTH - 3 && min === 0 ? [] : samples(def.element, depth + 1)
  const unique = [...new Map(items.map(item => [JSON.stringify(item), item])).values()]
  const full = unique.slice(0, Math.min(max, 6))
  while (full.length < min)
    full.push(unique[full.length % unique.length])
  const out: unknown[] = [full]
  if (min === 0 && full.length > 0)
    out.push([])
  return out
}

function keySamples(key: Schema): string[] {
  const def = defOf(key)
  if (def.type === 'enum')
    return Object.values(def.entries).map(String)
  const checks = checksOf(key)
  if (checks.length === 0)
    return ['alpha', 'beta', 'gamma']
  return stringSamples(key)
}

function recordSamples(def: z.core.$ZodRecordDef, depth: number): unknown[] {
  const keys = keySamples(def.keyType)
  const full = defOf(def.keyType).type === 'enum' && !(def as { partial?: boolean }).partial
  const values = depth > MAX_DEPTH - 3 ? [] : samples(def.valueType, depth + 1).filter(value => value !== ABSENT)
  if (values.length === 0) {
    if (full)
      throw new TooDeep()
    return [{}]
  }
  const used = (full ? keys : keys.slice(0, Math.max(1, Math.min(keys.length, values.length)))).sort()
  const record = Object.fromEntries(used.map((key, index) => [key, values[index % values.length]]))
  return full ? [record] : [record, {}]
}

function objectSamples(def: z.core.$ZodObjectDef, depth: number): unknown[] {
  const fields = Object.entries(def.shape).map(([key, child]) => {
    const found = samples(child, depth + 1)
    if (found.length === 0)
      throw new TooDeep()
    return [key, found] as const
  })
  const count = Math.max(1, ...fields.map(([, found]) => found.length))
  return Array.from({ length: count }, (_, index) => {
    const out: Record<string, unknown> = {}
    for (const [key, found] of fields) {
      const value = found[index % found.length]
      if (value !== ABSENT)
        out[key] = value
    }
    return out
  })
}

type Path = (string | number)[]
type Edit = { path: Path, kind: 'set', value: unknown } | { path: Path, kind: 'delete' }

interface Mutant {
  name: string
  edit: Edit
  /** A value only MessagePack can carry, such as NaN. */
  msgpackOnly?: boolean
}

/**
 * Edits of the sample that break one constraint each, once per schema node
 * and kind of breakage. Zod judges each; some are accepted (a stripping
 * object drops an unknown key).
 */
function mutants(root: Schema, sample: unknown, seen: WeakMap<object, Set<string>>): Mutant[] {
  const out: Mutant[] = []
  const once = (schema: Schema, kind: string, mutant: Mutant): void => {
    const kinds = seen.get(schema) ?? new Set<string>()
    seen.set(schema, kinds)
    if (kinds.has(kind))
      return
    kinds.add(kind)
    out.push(mutant)
  }
  const set = (path: Path, value: unknown, name: string, msgpackOnly = false): Mutant =>
    ({ name, edit: { path, kind: 'set', value }, msgpackOnly })
  const walk = (schema: Schema, value: unknown, path: Path): void => {
    const def = defOf(schema)
    const at = path.join('.') || 'root'
    switch (def.type) {
      case 'lazy':
        walk(resolveLazy(schema), value, path)
        return
      case 'optional':
        walk(def.innerType, value, path)
        return
      case 'nullable':
        if (value !== null)
          walk(def.innerType, value, path)
        return
      case 'union': {
        const option = unionOptions(schema).find(candidate => (candidate as z.ZodType).safeParse(value).success)
        once(schema, 'no-option', set(path, typeof value === 'object' ? 12345 : { type: 'bogus' }, `${at}: no union option`))
        if (option)
          walk(option, value, path)
        return
      }
      case 'object': {
        const object = value as Record<string, unknown>
        once(schema, 'unknown-key', set([...path, 'zzUnknown'], 1, `${at}: unknown key`))
        once(schema, 'not-object', set(path, 'text', `${at}: not an object`))
        once(schema, 'reordered', set(path, Object.fromEntries(Object.entries(object).reverse()), `${at}: keys reordered`))
        for (const [key, child] of Object.entries(def.shape)) {
          const childDef = defOf(child)
          if (childDef.type !== 'optional') {
            once(schema, `missing:${key}`, { name: `${at}: missing ${key}`, edit: { path: [...path, key], kind: 'delete' } })
          } else if (key in object && !['nullable', 'unknown'].includes(defOf(childDef.innerType).type)) {
            once(schema, `null:${key}`, set([...path, key], null, `${at}: null optional ${key}`))
          }
          if (key in object)
            walk(child, object[key], [...path, key])
        }
        return
      }
      case 'literal':
        once(schema, 'other', set(path, typeof value === 'string' ? 'bogus-literal' : 424242, `${at}: other literal`))
        return
      case 'enum':
        once(schema, 'bogus', set(path, 'bogus-member', `${at}: not a member`))
        once(schema, 'type', set(path, 7, `${at}: enum of wrong type`))
        return
      case 'string': {
        once(schema, 'type', set(path, 42, `${at}: not a string`))
        const { min, max } = lengthBounds(schema)
        if (min > 0)
          once(schema, 'short', set(path, 'x'.repeat(min - 1), `${at}: too short`))
        if (Number.isFinite(max) && max <= 100_000) {
          once(schema, 'long', set(path, 'x'.repeat(max + 1), `${at}: too long`))
          if (max <= 64)
            once(schema, 'long-astral', set(path, '😀'.repeat(max + 1), `${at}: too many code points`))
        }
        if (checksOf(schema).some(check => check.check === 'string_format'))
          once(schema, 'format', set(path, 'bad value!\u0000', `${at}: breaks its pattern`))
        if ('format' in def && def.format === 'url')
          once(schema, 'url', set(path, 'not a url', `${at}: not a URL`))
        return
      }
      case 'number': {
        const bounds = numberBounds(schema)
        once(schema, 'type', set(path, '1', `${at}: not a number`))
        once(schema, 'nan', set(path, Number.NaN, `${at}: NaN`, true))
        if (isInt(schema)) {
          once(schema, 'fraction', set(path, 1.5, `${at}: not an integer`))
          once(schema, 'unsafe', set(path, 2 ** 53, `${at}: unsafe integer`))
        }
        if (Number.isFinite(bounds.min))
          once(schema, 'below', set(path, bounds.minInclusive ? bounds.min - (isInt(schema) ? 1 : 0.5) : bounds.min, `${at}: below the minimum`))
        if (Number.isFinite(bounds.max))
          once(schema, 'above', set(path, bounds.maxInclusive ? bounds.max + (isInt(schema) ? 1 : 0.5) : bounds.max, `${at}: above the maximum`))
        return
      }
      case 'boolean':
        once(schema, 'type', set(path, 'true', `${at}: not a boolean`))
        return
      case 'custom':
        once(schema, 'type', set(path, 'AQID', `${at}: text instead of bytes`))
        return
      case 'date':
        once(schema, 'type', set(path, 1_700_000_000_000, `${at}: number instead of a date`))
        return
      case 'null':
        once(schema, 'type', set(path, 0, `${at}: not null`))
        return
      case 'array': {
        const items = value as unknown[]
        once(schema, 'type', set(path, { 0: items[0] }, `${at}: not an array`))
        const { min, max } = lengthBounds(schema)
        if (min > 0)
          once(schema, 'short', set(path, items.slice(0, min - 1), `${at}: too few items`))
        if (Number.isFinite(max) && max <= 1000 && items.length > 0)
          once(schema, 'long', set(path, Array.from({ length: max + 1 }, (_, index) => items[index % items.length]), `${at}: too many items`))
        if (checksOf(schema).some(check => check.check === 'custom') && items.length > 0)
          once(schema, 'duplicate', set(path, [...items, items[0]], `${at}: repeated item`))
        if (items.length > 0)
          walk(def.element, items[0], [...path, 0])
        return
      }
      case 'record': {
        const record = value as Record<string, unknown>
        const entries = Object.entries(record)
        const keyDef = defOf(def.keyType)
        if (entries.length > 0 && (keyDef.type === 'enum' || checksOf(def.keyType).length > 0))
          once(schema, 'bad-key', set([...path, 'bad key!'], entries[0]![1], `${at}: key breaks its schema`))
        if (keyDef.type === 'enum' && !(def as { partial?: boolean }).partial && entries.length > 0)
          once(schema, 'missing-key', { name: `${at}: missing record key`, edit: { path: [...path, entries[0]![0]], kind: 'delete' } })
        once(schema, 'type', set(path, [], `${at}: not an object`))
        if (entries.length > 0)
          walk(def.valueType, entries[0]![1], [...path, entries[0]![0]])
        return
      }
      case 'unknown':
        return
      default:
        throw new Error(`No mutants for ${def.type}`)
    }
  }
  walk(root, sample, [])
  return out
}

function applyEdit(root: unknown, edit: Edit): unknown {
  if (edit.path.length === 0)
    return edit.kind === 'set' ? edit.value : undefined
  const copy = structuredClone(root)
  let node = copy as Record<string | number, unknown>
  for (const key of edit.path.slice(0, -1))
    node = node[key] as Record<string | number, unknown>
  const last = edit.path.at(-1)!
  if (edit.kind === 'delete')
    delete node[last]
  else
    node[last] = edit.value
  return copy
}

function encode(codec: Codec, portable: boolean, value: unknown): string {
  if (codec === 'msgpack')
    return Buffer.from(msgpackCodec.encode(value)).toString('base64')
  return portable ? stringifyPortableJson(value) : JSON.stringify(value)
}

interface Case {
  root: string
  codec: Codec
  name: string
  input: string
  output: string | null
}

function record(contract: GoContractPackage): Case[] {
  const cases: Case[] = []
  for (const root of contract.roots) {
    const schema = root.schema as z.ZodType
    const values = samples(root.schema).filter(value => value !== ABSENT)
    const accepted = values.filter(value => schema.safeParse(value).success)
    if (accepted.length === 0)
      throw new Error(`${contract.name} ${root.name}: no sample passes its schema`)
    for (const [index, value] of values.entries()) {
      if (!schema.safeParse(value).success)
        console.warn(`${contract.name} ${root.name}: sample ${index} fails its schema: ${schema.safeParse(value).error?.issues[0]?.message}`)
    }
    const trials: { name: string, value: unknown, msgpackOnly?: boolean }[] = accepted.map((value, index) => ({ name: `sample ${index}`, value }))
    // Each schema node is broken once, in the richest sample that reaches it.
    const seen = new WeakMap<object, Set<string>>()
    const richestFirst = [...accepted].sort((first, second) => JSON.stringify(second).length - JSON.stringify(first).length)
    for (const sample of richestFirst) {
      for (const mutant of mutants(root.schema, sample, seen))
        trials.push({ name: mutant.name, value: applyEdit(sample, mutant.edit), msgpackOnly: mutant.msgpackOnly })
    }
    for (const codec of root.codecs) {
      for (const trial of trials) {
        if (trial.msgpackOnly && codec !== 'msgpack')
          continue
        const parsed = schema.safeParse(trial.value)
        cases.push({
          root: root.name,
          codec,
          name: trial.name,
          input: encode(codec, contract.portableJson, trial.value),
          output: parsed.success ? encode(codec, contract.portableJson, parsed.data) : null,
        })
      }
    }
  }
  return cases
}

if (import.meta.main) {
  const root = join(import.meta.dir, '..')
  for (const contract of goContractPackages()) {
    const cases = record(contract)
    const directory = join(root, 'internal/contract', contract.name, 'testdata')
    await mkdir(directory, { recursive: true })
    await writeFile(join(directory, 'corpus.json'), `${JSON.stringify({ cases }, null, 1)}\n`)
    const rejected = cases.filter(item => item.output === null).length
    console.log(`${contract.name}: ${cases.length} cases, ${rejected} rejected by Zod`)
  }
}
