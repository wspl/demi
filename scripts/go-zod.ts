import { z } from 'zod'

// Shared Zod-to-Go generation tooling, the counterpart of `rust-zod.ts`. It
// turns a Zod schema into Go types, a shape parser, a validator and a tree
// encoder that the runtime in `internal/contract/zodrt` completes with the
// JSON and MessagePack codecs. Unsupported Zod constructs fail generation.

type Schema = z.core.$ZodType
type Def = z.core.$ZodTypes['_zod']['def']

const INITIALISMS = new Set([
  'api', 'cpu', 'css', 'dns', 'eof', 'gid', 'html', 'http', 'https', 'id', 'ip', 'json', 'ok',
  'rpc', 'sql', 'tcp', 'tls', 'uid', 'uri', 'url', 'uuid', 'xml',
])

/** An exported Go name for a key, tag or constant: `deviceId` → `DeviceID`. */
export function goName(value: string): string {
  const words = value.replace(/([a-z0-9])([A-Z])/g, '$1 $2').split(/[^A-Za-z0-9]+/).filter(Boolean)
  if (words.length === 0)
    throw new Error(`No Go name for ${JSON.stringify(value)}`)
  return words.map(word => INITIALISMS.has(word.toLowerCase())
    ? word.toUpperCase()
    : `${word[0]!.toUpperCase()}${word.slice(1)}`).join('')
}

/** A Go string literal; JSON escapes are Go escapes. */
export function goString(value: string): string {
  return JSON.stringify(value)
}

/** A Go number literal for a JavaScript number. */
export function goNumber(value: number): string {
  if (!Number.isFinite(value))
    throw new Error(`No Go literal for ${value}`)
  return String(value)
}

/**
 * A Go declaration for an exported TypeScript constant: numbers and strings
 * become constants, string arrays a function returning a fresh slice, and an
 * object of numbers one constant per entry.
 */
export function goConstant(name: string, value: unknown): string {
  const constName = goName(name.toLowerCase())
  if (typeof value === 'number')
    return `const ${constName} = ${goNumber(value)}`
  if (typeof value === 'string')
    return `const ${constName} = ${goString(value)}`
  if (Array.isArray(value) && value.every(item => typeof item === 'string'))
    return `func ${constName}() []string {\nreturn []string{${value.map(goString).join(', ')}}\n}`
  if (typeof value === 'object' && value !== null && Object.values(value).every(item => typeof item === 'number'))
    return Object.entries(value).map(([key, item]) => `const ${constName}${goName(key)} = ${goNumber(item as number)}`).join('\n')
  throw new Error(`Unsupported constant ${name}`)
}

/** A type another generated package owns, which this package imports. */
export interface GoForeign {
  pkg: string
  path: string
  name: string
  kind: Kind
}

type Kind = 'struct' | 'union' | 'enum'

interface Options {
  /** The custom schema that stands for binary data (a Uint8Array). */
  bytes?: Schema
  /** Whether `z.date()` may appear; only codecs that carry dates allow it. */
  dates?: boolean
  foreign?: Map<Schema, GoForeign>
  /** Whether JSON carries bytes and dates as portable markers. */
  portableJson?: boolean
}

/** The object variants of a union, nested unions flattened. */
export function unionOptions(schema: Schema): Schema[] {
  const def = defOf(schema)
  if (def.type === 'union')
    return def.options.flatMap(unionOptions)
  return [schema]
}

function defOf(schema: Schema): Def {
  return (schema as z.core.$ZodTypes)._zod.def
}

/**
 * A lazy schema's target, which Zod resolves once and caches; `unwrap()`
 * would build a new one on every call.
 */
export function resolveLazy(schema: Schema): Schema {
  return (schema as z.core.$ZodLazy)._zod.innerType
}

/** The literal fields of an object that fix one value, in shape order. */
function constants(schema: Schema): [string, string | number | boolean][] {
  const def = defOf(schema)
  if (def.type !== 'object')
    return []
  return Object.entries(def.shape).flatMap(([key, child]) => {
    const value = constantValue(child)
    return value === undefined ? [] : [[key, value] as [string, string | number | boolean]]
  })
}

function constantValue(schema: Schema): string | number | boolean | undefined {
  const def = defOf(schema)
  if (def.type !== 'literal' || def.values.length !== 1)
    return undefined
  const value = def.values[0]
  if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean')
    throw new Error(`Unsupported literal ${String(value)}`)
  return value
}

function goConstantValue(value: string | number | boolean): string {
  if (typeof value === 'string')
    return goString(value)
  if (typeof value === 'number')
    return `float64(${goNumber(value)})`
  return String(value)
}

/** Whether a number schema is `.int()`, a safe integer. */
function isInt(schema: Schema): boolean {
  return (defOf(schema).checks ?? []).some(check => {
    const constraint = (check as z.core.$ZodChecks)._zod.def
    return constraint.check === 'number_format' && constraint.format === 'safeint'
  })
}

// JavaScript's `\s`: WhiteSpace and LineTerminator of ECMAScript.
const JS_WHITESPACE = String.raw`\t\n\v\f\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}`

/**
 * A JavaScript regular expression source in Go's RE2 syntax. The two agree
 * on most syntax; this rewrites what they mean differently (`\s`, `\S`, `.`,
 * `\0`, `\u`) and refuses what RE2 lacks.
 */
export function goPattern(source: string, flags: string): string {
  if (flags && flags !== 'u')
    throw new Error(`Unsupported regex flags: ${flags}`)
  let out = ''
  let inClass = false
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index]!
    if (char === '\\') {
      const next = source[index + 1]
      index += 1
      if (next === undefined)
        throw new Error(`Unsupported regex ${source}`)
      if (next === 's') {
        out += inClass ? JS_WHITESPACE : `[${JS_WHITESPACE}]`
      } else if (next === 'S') {
        if (inClass)
          throw new Error(`Unsupported regex ${source}: \\S in a class`)
        out += `[^${JS_WHITESPACE}]`
      } else if (next === '0' && !/[0-9]/.test(source[index + 1] ?? '')) {
        out += String.raw`\x00`
      } else if (next === 'u') {
        const braced = /^\{([0-9a-fA-F]+)\}/.exec(source.slice(index + 1))
        const plain = /^[0-9a-fA-F]{4}/.exec(source.slice(index + 1))
        if (braced && flags === 'u') {
          out += `\\x{${braced[1]}}`
          index += braced[0].length
        } else if (plain) {
          out += `\\x{${plain[0]}}`
          index += 4
        } else {
          throw new Error(`Unsupported regex ${source}`)
        }
      } else if (/[dDwWnrtfvx]/.test(next) || (next === 'b' && !inClass) || (next === 'B' && !inClass)) {
        out += `\\${next}`
      } else if (/[\^$\\.*+?()[\]{}|/-]/.test(next)) {
        out += `\\${next}`
      } else {
        throw new Error(`Unsupported regex escape \\${next} in ${source}`)
      }
    } else if (char === '[' && !inClass) {
      inClass = true
      out += char
      if (source[index + 1] === '^') {
        out += '^'
        index += 1
      }
      if (source[index + 1] === ']')
        throw new Error(`Unsupported regex ${source}: empty or leading ] class`)
    } else if (char === ']' && inClass) {
      inClass = false
      out += char
    } else if (char === '.' && !inClass) {
      out += String.raw`[^\n\r\x{2028}\x{2029}]`
    } else if (char === '(' && source[index + 1] === '?' && source[index + 2] !== ':') {
      throw new Error(`Unsupported regex group in ${source}`)
    } else {
      out += char
    }
  }
  return out
}

/** Generates Go types, parsers, validators and encoders from Zod schemas. */
export class GoZodTypes {
  private readonly declarations: string[] = []
  private readonly names = new Map<Schema, string>()
  private readonly kinds = new Map<string, Kind>()
  private readonly preset = new Map<Schema, string>()
  private readonly taken = new Map<string, Schema>()
  private readonly active = new Set<Schema>()
  private readonly markers = new Map<string, string[]>()
  private readonly patterns = new Map<string, string>()
  private readonly prefixes = new Map<Schema, string>()
  /** The import paths of the foreign types the generated code uses. */
  readonly imports = new Set<string>()

  constructor(private readonly options: Options = {}) {}

  /** Names a schema before it is first met, so it is declared under that name. */
  name(schema: Schema, name: string): this {
    this.preset.set(schema, name)
    return this
  }

  /** Names the variants of a union after this prefix and their tags. */
  variantPrefix(schema: Schema, prefix: string): this {
    this.prefixes.set(schema, prefix)
    return this
  }

  /** The Go type of a schema, declaring the named types it needs. */
  type(schema: Schema, name: string, indirect = false): string {
    const foreign = this.options.foreign?.get(schema)
    if (foreign) {
      this.imports.add(foreign.path)
      return `${foreign.pkg}.${foreign.name}`
    }
    const known = this.names.get(schema)
    if (known)
      return !indirect && this.active.has(schema) ? `*${known}` : known
    const def = defOf(schema)
    switch (def.type) {
      case 'string': return 'string'
      case 'boolean': return 'bool'
      case 'number': return isInt(schema) ? 'int64' : 'float64'
      case 'custom':
        if (schema === this.options.bytes)
          return '[]byte'
        throw new Error('A custom Zod type needs an explicit native representation')
      case 'date':
        if (!this.options.dates)
          throw new Error('No native date representation configured')
        return 'time.Time'
      case 'unknown': return 'any'
      case 'null': return 'zodrt.Null'
      case 'literal': return this.literalType(schema)
      case 'enum': return this.declareEnum(schema, name)
      case 'optional':
        throw new Error('An optional schema is only supported as an object field or record value')
      case 'nullable': return `zodrt.Nullable[${this.type(def.innerType, name, indirect)}]`
      case 'array': return `[]${this.type(def.element, `${name}Item`, true)}`
      case 'record': {
        const key = this.type(def.keyType, `${name}Key`, true)
        if (key !== 'string' && this.kindOf(def.keyType) !== 'enum')
          throw new Error(`Unsupported record key type ${key}`)
        return `map[${key}]${this.type(recordValue(def), `${name}Value`, true)}`
      }
      case 'lazy': {
        this.names.set(schema, this.preset.get(schema) ?? name)
        const type = this.type(resolveLazy(schema), this.names.get(schema)!, indirect)
        if (!this.kinds.has(type.replace(/^\*/, '')))
          throw new Error('A lazy schema must resolve to an object or a union')
        this.names.set(schema, type.replace(/^\*/, ''))
        return type
      }
      case 'object': return this.declareObject(schema, name)
      case 'union': return this.declareUnion(schema, name)
      default: throw new Error(`Unsupported Zod Go type: ${def.type}`)
    }
  }

  /** The kind of a named type, or undefined for a structural one. */
  private kindOf(schema: Schema): Kind | undefined {
    const foreign = this.options.foreign?.get(schema)
    if (foreign)
      return foreign.kind
    const name = this.names.get(schema)
    return name ? this.kinds.get(name) : undefined
  }

  private literalType(schema: Schema): string {
    const def = defOf(schema) as z.core.$ZodLiteralDef<z.core.util.Literal>
    const types = new Set(def.values.map(value => typeof value === 'number'
      ? (Number.isInteger(value) ? 'int64' : 'float64')
      : typeof value === 'string' ? 'string' : typeof value === 'boolean' ? 'bool' : 'unsupported'))
    if (types.size !== 1 || types.has('unsupported'))
      throw new Error('A literal must hold strings, numbers or booleans of one type')
    return [...types][0]!
  }

  private claim(schema: Schema, suggested: string, kind: Kind): string {
    const name = this.preset.get(schema) ?? suggested
    const owner = this.taken.get(name)
    if (owner && owner !== schema)
      throw new Error(`Go name collision: ${name}`)
    this.taken.set(name, schema)
    this.names.set(schema, name)
    this.kinds.set(name, kind)
    return name
  }

  /** A shape parser: a Go expression of type zodrt.Parser[T]. */
  parser(schema: Schema, indirect = false): string {
    const foreign = this.options.foreign?.get(schema)
    if (foreign)
      return `${foreign.pkg}.Parse${foreign.name}`
    const known = this.names.get(schema)
    if (known)
      return !indirect && this.active.has(schema) ? `zodrt.Pointer(parse${known})` : `parse${known}`
    const def = defOf(schema)
    switch (def.type) {
      case 'string': return 'zodrt.String'
      case 'boolean': return 'zodrt.Bool'
      case 'number': return isInt(schema) ? 'zodrt.Int' : 'zodrt.Number'
      case 'custom': return 'zodrt.Bytes'
      case 'date': return 'zodrt.Date'
      case 'unknown': return 'zodrt.Unknown'
      case 'null': return 'zodrt.ParseNull'
      case 'literal': return {
        int64: 'zodrt.Int', float64: 'zodrt.Number', string: 'zodrt.String', bool: 'zodrt.Bool',
      }[this.literalType(schema)]!
      case 'nullable': return `zodrt.NullableOf(${this.parser(def.innerType, indirect)})`
      case 'array': return `zodrt.Array(${this.parser(def.element, true)})`
      case 'record': return `zodrt.Record(${this.parser(def.keyType, true)}, ${this.parser(recordValue(def), true)})`
      default: throw new Error(`No parser for an unnamed ${def.type}`)
    }
  }

  /**
   * Statements that return an error when the value `value` (a Go expression
   * of this schema's type) breaks a constraint of the schema.
   */
  validate(schema: Schema, value: string, indirect = false): string {
    const kind = this.kindOf(schema)
    if (kind) {
      const pointer = !indirect && this.active.has(schema)
      const nilCheck = kind === 'union' || pointer
        ? `if ${value} == nil {\nreturn zodrt.Invalid("required")\n}\n`
        : ''
      return `${nilCheck}if err := ${value}.Validate(); err != nil {\nreturn err\n}`
    }
    const def = defOf(schema)
    const lines: string[] = []
    const require = (condition: string, message: string): void => {
      lines.push(`if !(${condition}) {\nreturn zodrt.Invalid(${goString(message)})\n}`)
    }
    switch (def.type) {
      case 'nullable': {
        const inner = this.validate(def.innerType, `${value}.Value`, indirect)
        if (inner)
          lines.push(`if ${value}.Valid {\n${inner}\n}`)
        break
      }
      case 'literal': {
        const choices = def.values.map(item => typeof item === 'string' ? goString(item) : String(item))
        require(choices.map(choice => `${value} == ${choice}`).join(' || '), 'invalid literal')
        break
      }
      case 'array': {
        const inner = this.validate(def.element, 'v', true)
        if (inner)
          lines.push(this.each(`index, item := range ${value}`, 'item', this.type(def.element, 'Item', true), inner, 'zodrt.AtIndex(index, err)'))
        break
      }
      case 'record': {
        const keys = this.validate(def.keyType, 'v', true)
        const values = this.validate(recordValue(def), 'v', true)
        if (keys || values) {
          const key = this.type(def.keyType, 'Key', true)
          const body = [
            keys ? this.each('', 'key', key, keys, 'zodrt.At(string(key), zodrt.Invalid("invalid key: %s", err))') : '',
            values ? this.each('', `${value}[key]`, this.type(recordValue(def), 'Value', true), values, 'zodrt.At(string(key), err)') : '',
          ].filter(Boolean).join('\n')
          lines.push(`for _, key := range slices.Sorted(maps.Keys(${value})) {\n${body}\n}`)
        }
        // A partial record names any of its keys; a full one names each.
        if (this.kindOf(def.keyType) === 'enum' && !(def as { partial?: boolean }).partial)
          require(`len(${value}) == ${Object.keys((defOf(def.keyType) as z.core.$ZodEnumDef).entries).length}`, 'missing record key')
        break
      }
      case 'number':
        lines.push(isInt(schema)
          ? `if err := zodrt.CheckSafeInt(${value}); err != nil {\nreturn err\n}`
          : `if err := zodrt.CheckFinite(${value}); err != nil {\nreturn err\n}`)
        break
      case 'string':
        if ('format' in def && def.format !== undefined) {
          if (def.format !== 'url')
            throw new Error(`Unsupported string format ${def.format}`)
          lines.push(`if err := zodrt.CheckURL(${value}); err != nil {\nreturn err\n}`)
        }
        break
      case 'date':
        lines.push(`if err := zodrt.CheckDate(${value}); err != nil {\nreturn err\n}`)
        break
      case 'custom':
        if (schema !== this.options.bytes)
          throw new Error('Unsupported custom Zod validation')
        break
      case 'boolean':
      case 'unknown':
      case 'null': break
      default: throw new Error(`Unsupported Zod validation: ${def.type}`)
    }
    for (const check of def.checks ?? []) {
      const constraint = (check as z.core.$ZodChecks | z.core.$ZodCustom)._zod.def
      switch (constraint.check) {
        case 'number_format':
          if (constraint.format !== 'safeint')
            throw new Error(`Unsupported number format ${constraint.format}`)
          break
        case 'greater_than':
        case 'less_than': {
          const bound = Number(constraint.value)
          const operator = `${constraint.check === 'greater_than' ? '>' : '<'}${constraint.inclusive ? '=' : ''}`
          const operand = isInt(schema) && !Number.isInteger(bound) ? `float64(${value})` : value
          require(`${operand} ${operator} ${goNumber(bound)}`, 'number out of range')
          break
        }
        case 'min_length':
        case 'max_length':
        case 'length_equals': {
          const length = def.type === 'string' ? `zodrt.Length(${value})` : `len(${value})`
          const [operator, bound] = constraint.check === 'min_length' ? ['>=', constraint.minimum]
            : constraint.check === 'max_length' ? ['<=', constraint.maximum] : ['==', constraint.length]
          require(`${length} ${operator} ${bound}`, 'invalid length')
          break
        }
        case 'string_format': {
          if (constraint.format !== 'regex')
            throw new Error(`Unsupported string format ${constraint.format}`)
          const pattern = (constraint as z.core.$ZodCheckRegexDef).pattern
          const source = goPattern(pattern.source, pattern.flags)
          let name = this.patterns.get(source)
          if (!name) {
            name = `pattern${this.patterns.size}`
            this.patterns.set(source, name)
          }
          require(`${name}.MatchString(${value})`, 'invalid string format')
          break
        }
        case 'custom':
          if (z.globalRegistry.get(schema)?.uniqueItems !== true || def.type !== 'array'
            || def.checks?.filter(item => item._zod.def.check === 'custom').length !== 1)
            throw new Error('Custom Zod refinements need a supported declarative constraint')
          lines.push(`if err := zodrt.Unique(${value}); err != nil {\nreturn err\n}`)
          break
        default: throw new Error(`Unsupported Zod check: ${constraint.check}`)
      }
    }
    return lines.join('\n')
  }

  /** A loop body that runs checks on one item and places their error. */
  private each(range: string, item: string, type: string, checks: string, place: string): string {
    const direct = checks === 'if err := v.Validate(); err != nil {\nreturn err\n}'
    const call = direct ? `if err := ${item}.Validate(); err != nil {\nreturn ${place}\n}` : `if err := func(v ${type}) error {\n${checks}\nreturn nil\n}(${item}); err != nil {\nreturn ${place}\n}`
    return range ? `for ${range} {\n${call}\n}` : call
  }

  /** A Go expression that encodes the value `value` as a tree. */
  encoder(schema: Schema, value: string, indirect = false): string {
    if (this.kindOf(schema))
      return `${value}.ToValue()`
    const def = defOf(schema)
    switch (def.type) {
      case 'null': return 'nil'
      case 'nullable': return `zodrt.NullableValue(${value}, func(v ${this.type(def.innerType, 'Value', indirect)}) any {\nreturn ${this.encoder(def.innerType, 'v', indirect)}\n})`
      case 'array': return `zodrt.ArrayValue(${value}, func(v ${this.type(def.element, 'Item', true)}) any {\nreturn ${this.encoder(def.element, 'v', true)}\n})`
      case 'record': return `zodrt.RecordValue(${value}, func(v ${this.type(recordValue(def), 'Value', true)}) any {\nreturn ${this.encoder(recordValue(def), 'v', true)}\n})`
      default: return value
    }
  }

  private declareEnum(schema: Schema, suggested: string): string {
    const def = defOf(schema) as z.core.$ZodEnumDef
    const name = this.claim(schema, suggested, 'enum')
    const values = Object.values(def.entries)
    if (!values.every(value => typeof value === 'string'))
      throw new Error(`Enum ${name} must hold strings`)
    const consts = values.map(value => `${name}${goName(value as string)}`)
    if (new Set(consts).size !== consts.length)
      throw new Error(`Enum ${name} has colliding constant names`)
    this.declarations.push(`type ${name} string

const (
${values.map((value, index) => `${consts[index]} ${name} = ${goString(value as string)}`).join('\n')}
)

func parse${name}(value any) (${name}, error) {
text, err := zodrt.String(value)
return ${name}(text), err
}

// Parse${name} parses and validates a decoded ${name}.
func Parse${name}(value any) (${name}, error) {
return zodrt.Try(parse${name}, value)
}

// Validate reports whether the value is one of the enum's members.
func (x ${name}) Validate() error {
switch x {
case ${consts.join(', ')}:
return nil
}
return zodrt.Invalid("invalid enum value %q", string(x))
}

// ToValue encodes the value as a tree.
func (x ${name}) ToValue() any {
return string(x)
}`)
    return name
  }

  private declareObject(schema: Schema, suggested: string): string {
    const def = defOf(schema) as z.core.$ZodObjectDef
    const catchall = def.catchall ? defOf(def.catchall).type : undefined
    if (catchall !== undefined && catchall !== 'never')
      throw new Error('Only strict and stripping Zod objects are supported')
    const mode = catchall === 'never' ? 'zodrt.Strict' : 'zodrt.Strip'
    const name = this.claim(schema, suggested, 'struct')
    this.active.add(schema)
    const members: string[] = []
    const parses: string[] = []
    const checks: string[] = []
    const values: string[] = []
    const entries = Object.entries(def.shape)
    for (const [key, child] of entries) {
      const constant = constantValue(child)
      if (constant !== undefined) {
        parses.push(`if err := zodrt.Constant(fields, ${goString(key)}, ${goConstantValue(constant)}); err != nil {\nreturn ${name}{}, err\n}`)
        values.push(`out = append(out, zodrt.Field{Key: ${goString(key)}, Value: ${goConstantValue(constant)}})`)
        continue
      }
      const field = goName(key)
      if (field === 'Validate' || field === 'ToValue')
        throw new Error(`Field ${key} of ${name} collides with a generated method`)
      const optional = defOf(child).type === 'optional'
      const inner = optional ? (defOf(child) as z.core.$ZodOptionalDef).innerType : child
      const type = this.type(inner, `${name}${field}`)
      const description = z.globalRegistry.get(child)?.description ?? z.globalRegistry.get(inner)?.description
      members.push(`${description ? `// ${description}\n` : ''}${field} ${optional ? `zodrt.Optional[${type}]` : type}`)
      const parser = this.parser(inner)
      parses.push(`if out.${field}, err = zodrt.${optional ? 'OptionalField' : 'Required'}(fields, ${goString(key)}, ${parser}); err != nil {\nreturn ${name}{}, err\n}`)
      const access = optional ? `x.${field}.Value` : `x.${field}`
      const check = this.validate(inner, 'v')
      if (check) {
        const call = this.each('', access, type, check, `zodrt.At(${goString(key)}, err)`)
        checks.push(optional ? `if x.${field}.Present {\n${call}\n}` : call)
      }
      const append = `out = append(out, zodrt.Field{Key: ${goString(key)}, Value: ${this.encoder(inner, access)}})`
      values.push(optional ? `if x.${field}.Present {\n${append}\n}` : append)
    }
    this.active.delete(schema)
    const hasFields = entries.some(([, child]) => constantValue(child) === undefined)
    const fieldsVar = hasFields || parses.length > 0 ? 'fields' : '_'
    this.declarations.push(`type ${name} struct {
${members.join('\n')}
}

var keys${name} = []string{${entries.map(([key]) => goString(key)).join(', ')}}

func parse${name}(value any) (${name}, error) {
${hasFields ? `var out ${name}\n` : ''}${fieldsVar}, err := zodrt.Fields(value, ${mode}, keys${name})
if err != nil {
return ${name}{}, err
}
${parses.join('\n')}
return ${hasFields ? 'out' : `${name}{}`}, nil
}

// Parse${name} parses and validates a decoded ${name}.
func Parse${name}(value any) (${name}, error) {
return zodrt.Try(parse${name}, value)
}

// Validate checks every constraint of the schema.
func (x ${name}) Validate() error {
${checks.join('\n')}
return nil
}

// ToValue encodes the value as a tree, fields in schema order.
func (x ${name}) ToValue() any {
out := make(zodrt.Object, 0, ${entries.length})
${values.join('\n')}
return out
}`)
    return name
  }

  private declareUnion(schema: Schema, suggested: string): string {
    const name = this.claim(schema, suggested, 'union')
    const options = unionOptions(schema)
    const prefix = this.prefixes.get(schema) ?? name
    const variants = options.map((option, index) => {
      if (this.options.foreign?.has(option))
        throw new Error(`Union ${name} cannot hold the foreign type of its option ${index}`)
      if (defOf(option).type === 'object') {
        const tags = constants(option).map(([, value]) => goName(String(value)))
        const variant = this.type(option, tags.length > 0 ? `${prefix}${tags.join('')}` : `${name}Variant${index}`)
        return variant
      }
      return this.declareWrapper(option, this.preset.get(option) ?? `${name}Variant${index}`)
    })
    for (const variant of variants)
      this.markers.set(variant, [...this.markers.get(variant) ?? [], name])
    const tries = variants.map(variant => `try${name}${variant}`)
    const adapters = variants.map((variant, index) => `func ${tries[index]}(value any) (${name}, error) {
x, err := zodrt.Try(parse${variant}, value)
if err != nil {
return nil, err
}
return x, nil
}`)
    const discriminator = this.discriminator(schema, options)
    let body: string
    if (discriminator) {
      const groups = new Map<string, string[]>()
      options.forEach((option, index) => {
        const tag = String(constants(option).find(([key]) => key === discriminator)![1])
        groups.set(tag, [...groups.get(tag) ?? [], tries[index]!])
      })
      body = `switch zodrt.Tag(value, ${goString(discriminator)}) {
${[...groups].map(([tag, candidates]) => `case ${goString(tag)}:\nreturn zodrt.FirstOf(value, ${candidates.join(', ')})`).join('\n')}
}
return nil, zodrt.At(${goString(discriminator)}, zodrt.Invalid("invalid discriminator"))`
    } else {
      body = `return zodrt.FirstOf(value, ${tries.join(', ')})`
    }
    this.declarations.push(`// ${name} is one of: ${variants.join(', ')}.
type ${name} interface {
is${name}()
Validate() error
ToValue() any
}

func parse${name}(value any) (${name}, error) {
${body}
}

// Parse${name} parses and validates a decoded ${name}.
func Parse${name}(value any) (${name}, error) {
return parse${name}(value)
}

${adapters.join('\n\n')}`)
    return name
  }

  /**
   * The key whose literal names each option: the discriminated union's own,
   * or the first literal key every option fixes. A plain union whose options
   * share no such key is tried option by option, as Zod does.
   */
  private discriminator(schema: Schema, options: Schema[]): string | undefined {
    const tagged = options.map(option => new Map(constants(option).filter(([, value]) => typeof value === 'string')))
    const def = defOf(schema)
    const declared = def.type === 'union' && 'discriminator' in def ? String(def.discriminator) : undefined
    const candidates = declared ? [declared] : [...tagged[0]?.keys() ?? []]
    return candidates.find(key => tagged.every(tags => tags.has(key)))
  }

  /** A named Go type for a union option that is not an object. */
  private declareWrapper(schema: Schema, name: string): string {
    const owner = this.taken.get(name)
    if (owner && owner !== schema)
      throw new Error(`Go name collision: ${name}`)
    this.taken.set(name, schema)
    const inner = this.type(schema, `${name}Value`, true)
    const check = this.validate(schema, 'v', true)
    this.declarations.push(`type ${name} ${inner}

func parse${name}(value any) (${name}, error) {
inner, err := ${this.parser(schema, true)}(value)
return ${name}(inner), err
}

// Validate checks every constraint of the schema.
func (x ${name}) Validate() error {
${check ? `return func(v ${inner}) error {\n${check}\nreturn nil\n}(${inner}(x))` : 'return nil'}
}

// ToValue encodes the value as a tree.
func (x ${name}) ToValue() any {
return ${this.encoder(schema, `${inner}(x)`, true)}
}`)
    return name
  }

  /**
   * Codec functions for a named root type: decode parses and validates at
   * once; encode refuses an invalid value.
   */
  root(schema: Schema, name: string, codecs: ('json' | 'msgpack')[]): string {
    const type = this.type(schema, name)
    const kind = this.kindOf(schema)
    if (!kind || type !== name)
      throw new Error(`Root ${name} must be a type declared in this package under that name, not ${type}`)
    const zero = kind === 'union' ? 'nil' : kind === 'enum' ? '""' : `${type}{}`
    const nilCheck = kind === 'union' ? `if x == nil {\nreturn nil, zodrt.Invalid("missing value")\n}\n` : ''
    return codecs.map(codec => {
      const [label, decode, encode] = codec === 'json'
        ? ['JSON', `zodrt.DecodeJSON(data, ${Boolean(this.options.portableJson)})`, `zodrt.EncodeJSON(x.ToValue(), ${Boolean(this.options.portableJson)})`]
        : ['Msgpack', 'zodrt.DecodeMsgpack(data)', 'zodrt.EncodeMsgpack(x.ToValue())']
      return `// Decode${type}${label} decodes ${label} and validates it as a ${type}.
func Decode${type}${label}(data []byte) (${type}, error) {
value, err := ${decode}
if err != nil {
return ${zero}, err
}
return Parse${type}(value)
}

// Encode${type}${label} validates a ${type} and encodes it as ${label}.
func Encode${type}${label}(x ${type}) ([]byte, error) {
${nilCheck}if err := x.Validate(); err != nil {
return nil, err
}
return ${encode}
}`
    }).join('\n\n')
  }

  /**
   * Parse and encode functions for a schema that is not a declared type, such
   * as a nullable result: `Parse<name>` and `<name>Value`.
   */
  valueRoot(schema: Schema, name: string): string {
    const type = this.type(schema, name)
    const check = this.validate(schema, 'v')
    const validate = check ? `if err := func(v ${type}) error {\n${check}\nreturn nil\n}(out); err != nil {\nreturn zero, err\n}\n` : ''
    return `// Parse${name} parses and validates a decoded ${name}.
func Parse${name}(value any) (${type}, error) {
var zero ${type}
out, err := ${this.parser(schema)}(value)
if err != nil {
return zero, err
}
${validate}return out, nil
}

// ${name}Value validates a ${name} and encodes it as a tree.
func ${name}Value(v ${type}) (any, error) {
${check ? `if err := func() error {\n${check}\nreturn nil\n}(); err != nil {\nreturn nil, err\n}\n` : ''}return ${this.encoder(schema, 'v')}, nil
}`
  }

  /** The generated declarations, union markers and patterns. */
  finish(): string {
    const markers = [...this.markers].flatMap(([type, unions]) =>
      unions.map(union => `func (${type}) is${union}() {}`))
    const patterns = [...this.patterns].map(([source, name]) =>
      `var ${name} = regexp.MustCompile(${goString(source)})`)
    return [...this.declarations, ...markers, ...patterns].join('\n\n')
  }
}

function recordValue(def: z.core.$ZodRecordDef): Schema {
  // A present key always has a value: neither codec can hold an undefined
  // one, so an optional record value is its inner schema on the wire.
  const value = def.valueType
  return defOf(value).type === 'optional' ? (defOf(value) as z.core.$ZodOptionalDef).innerType : value
}
