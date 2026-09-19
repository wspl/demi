import { z } from 'zod'

export function rustPascal(value: string): string {
  return value.replace(/(^|[_-])([a-z])/g, (_, _prefix, letter: string) => letter.toUpperCase())
}

export function rustField(value: string): string {
  const name = value.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replaceAll('-', '_').toLowerCase()
  return ['type', 'ref', 'self', 'match', 'loop', 'move', 'mod', 'fn'].includes(name) ? `r#${name}` : name
}

export function rustString(value: string): string {
  return JSON.stringify(value).replace(/\\u([0-9a-f]{4})/gi, '\\u{$1}')
}

interface Options {
  boxedUnions?: boolean
  overrides?: Map<z.core.$ZodType, string>
  bytes?: z.core.$ZodType
  dateType?: string
  jsonObjects?: Set<z.core.$ZodType>
}

/** The object variants of a union, nested unions flattened. */
export function unionOptions(schema: z.core.$ZodType): z.ZodObject[] {
  const def = (schema as z.core.$ZodTypes)._zod.def
  if (def.type === 'union')
    return def.options.flatMap(unionOptions)
  if (def.type !== 'object')
    throw new Error('Tagged variants must be objects')
  return [schema as z.ZodObject]
}

/** Generates serde types and native value checks from the owning Zod schemas. */
export class RustZodTypes {
  readonly declarations = new Map<string, string>()
  private readonly names = new Map<z.core.$ZodType, string>()
  private readonly activeObjects = new Set<z.core.$ZodType>()
  private readonly deserializers: string[] = []
  private readonly patterns = new Map<string, string>()

  constructor(private readonly options: Options = {}) {}

  fields(schema: z.ZodObject, name: string, skip = new Set<string>(), deserialize = true): string {
    if (schema._zod.def.catchall?._zod.def.type !== 'never')
      throw new Error('Fixed native contracts require strict Zod objects')
    return Object.entries(schema.shape).filter(([field]) => !skip.has(field)).map(([field, child]) => {
      const type = this.type(child, `${name}${rustPascal(field)}`)
      const optional = child._zod.def.type === 'optional'
      const validation = this.validate(child, '&value', false)
      let deserializeAttribute = ''
      if (deserialize && (validation || child._zod.def.type === 'nullable' || optional)) {
        const functionName = `deserialize_field_${this.deserializers.length}`
        const decodeType = optional ? this.type(child._zod.def.innerType, `${name}${rustPascal(field)}`) : type
        const decode = `<${decodeType} as serde::Deserialize>::deserialize(deserializer)?`
        const checks = validation ? `let checked: Result<(), String> = (|| { ${validation}\nOk(()) })();
          checked.map_err(serde::de::Error::custom)?;` : ''
        this.deserializers.push(`fn ${functionName}<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<${type}, D::Error> {
          let value = ${optional ? `Some(${decode})` : decode};
          ${checks}
          Ok(value)
        }`)
        deserializeAttribute = `, deserialize_with = "${functionName}"`
      }
      const defaultAttribute = optional ? ', default, skip_serializing_if = "Option::is_none"' : ''
      return `#[serde(rename = ${rustString(field)}${deserializeAttribute}${defaultAttribute})]\npub ${rustField(field)}: ${type},`
    }).join('\n')
  }

  type(schema: z.core.$ZodType, name: string, indirect = false): string {
    const known = this.options.overrides?.get(schema) ?? this.names.get(schema)
    if (known)
      return !indirect && this.activeObjects.has(schema) ? `Box<${known}>` : known
    const def = (schema as z.core.$ZodTypes)._zod.def
    const derive = '#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]'
    switch (def.type) {
      case 'string':
      case 'enum': return 'String'
      case 'boolean': return 'bool'
      case 'number': {
        const bag = (schema as z.core.$ZodNumber)._zod.bag
        if (bag.format !== 'safeint')
          return 'f64'
        const minimum = Math.max(bag.minimum ?? -Infinity, bag.exclusiveMinimum ?? -Infinity)
        if (minimum < 0)
          return 'i64'
        return bag.maximum !== undefined && bag.maximum <= 255 ? 'u8' : 'u64'
      }
      case 'custom':
        if (schema === this.options.bytes)
          return 'super::WireBytes'
        throw new Error('A custom Zod type needs an explicit native representation')
      case 'date':
        if (!this.options.dateType)
          throw new Error('No native date representation configured')
        return this.options.dateType
      case 'unknown': return 'serde_json::Value'
      case 'null': return '()'
      case 'literal': {
        const first = def.values[0]
        return typeof first === 'number' ? (first < 0 ? 'i64' : 'u64')
          : typeof first === 'boolean' ? 'bool' : 'String'
      }
      case 'optional':
      case 'nullable': return `Option<${this.type(def.innerType, name, indirect)}>`
      case 'array': return `Vec<${this.type(def.element, `${name}Item`, true)}>`
      case 'record': return `std::collections::BTreeMap<String, ${this.type(def.valueType, `${name}Value`, true)}>`
      case 'lazy': {
        this.names.set(schema, name)
        const resolved = def.getter()
        const type = this.type(resolved, name)
        this.names.set(schema, type)
        return type
      }
      case 'object': {
        this.names.set(schema, name)
        this.activeObjects.add(schema)
        const fields = this.fields(schema as z.ZodObject, name)
        this.activeObjects.delete(schema)
        this.declarations.set(name, `${derive}\n#[serde(deny_unknown_fields)]\npub struct ${name} {\n${fields}\n}`)
        return name
      }
      case 'union': {
        this.names.set(schema, name)
        const variants = def.options.map((child, index) => {
          let type = this.type(child, `${name}Variant${index}`)
          if (this.options.boxedUnions)
            type = `Box<${type}>`
          return `Variant${index}(${type}),`
        }).join('\n')
        this.declarations.set(name, `${derive}\n#[serde(untagged)]\npub enum ${name} {\n${variants}\n}`)
        return name
      }
      default: throw new Error(`Unsupported Zod Rust type: ${def.type}`)
    }
  }

  /** Checks native values without serializing them to JSON. */
  validate(schema: z.core.$ZodType, expression: string, recurseObjects = true): string {
    const def = (schema as z.core.$ZodTypes)._zod.def
    const value = /^[a-zA-Z_]\w*$/.test(expression) ? expression : `(${expression})`
    const lines: string[] = []
    const require = (condition: string, message: string): void => {
      lines.push(`if !(${condition}) { return Err(${rustString(message)}.into()); }`)
    }
    if (this.options.jsonObjects?.has(schema)) {
      return `if !${value}.is_object() { return Err("expected object".into()); }`
    }
    if (this.options.overrides?.has(schema)) {
      // External generated types validate when deserialized by their owning module.
      return ''
    }
    switch (def.type) {
      case 'optional':
      case 'nullable': {
        const inner = this.validate(def.innerType, 'value', recurseObjects)
        return inner ? `if let Some(value) = ${expression} { ${inner} }` : ''
      }
      case 'lazy':
        if (!recurseObjects)
          return ''
        return `${rustField(this.type(schema, 'Recursive'))}_validate(${value})?;`
      case 'object':
        if (recurseObjects) {
          for (const [field, child] of Object.entries((schema as z.ZodObject).shape))
            lines.push(this.validate(child, `&${value}.${rustField(field)}`, recurseObjects))
        }
        break
      case 'union':
        if (recurseObjects) {
          const type = this.type(schema, 'Union')
          const checks = def.options.map(child => this.validate(child, 'value', recurseObjects))
          if (checks.some(Boolean))
            lines.push(`match ${expression} { ${checks.map((check, index) =>
              `${type}::Variant${index}(${check ? 'value' : '_'}) => { ${check} },`).join('\n')} }`)
        }
        break
      case 'literal': {
        const choices = def.values.map(literal => typeof literal === 'string' ? rustString(literal) : String(literal))
        const actual = typeof def.values[0] === 'string' ? `${value}.as_str()` : `*${value}`
        require(`matches!(${actual}, ${choices.join(' | ')})`, 'invalid literal')
        break
      }
      case 'enum':
        require(`matches!(${value}.as_str(), ${Object.values(def.entries).map(item => rustString(String(item))).join(' | ')})`, 'invalid enum value')
        break
      case 'array': {
        const inner = this.validate(def.element, 'value', recurseObjects)
        if (inner)
          lines.push(`for value in ${expression} { ${inner} }`)
        break
      }
      case 'record': {
        const keys = this.validate(def.keyType, 'key', recurseObjects)
        const values = this.validate(def.valueType, 'value', recurseObjects)
        if (keys && values)
          lines.push(`for (key, value) in ${expression} { ${keys}\n${values} }`)
        else if (keys)
          lines.push(`for key in ${value}.keys() { ${keys} }`)
        else if (values)
          lines.push(`for value in ${value}.values() { ${values} }`)
        if (def.keyType._zod.def.type === 'enum')
          require(`${value}.len() == ${Object.keys((def.keyType as z.core.$ZodEnum)._zod.def.entries).length}`, 'missing record key')
        break
      }
      case 'number':
        if (this.type(schema, 'Number') === 'f64')
          require(`${value}.is_finite()`, 'number must be finite')
        break
      case 'string':
        if ('format' in def && def.format === 'url')
          require(`url::Url::parse(${expression}).is_ok()`, 'invalid URL')
        break
      case 'date':
        require(`${value}.0.unsigned_abs() <= 8_640_000_000_000_000`, 'date out of range')
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
          if (this.type(schema, 'Number') === 'i64')
            require(`(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(${expression})`, 'integer out of range')
          else if (this.type(schema, 'Number') === 'u64')
            require(`*${value} <= 9_007_199_254_740_991`, 'integer out of range')
          break
        case 'greater_than':
        case 'less_than': {
          const nativeType = this.type(schema, 'Number')
          if (constraint.inclusive && ((constraint.value === 0 && constraint.check === 'greater_than' && nativeType !== 'f64' && nativeType !== 'i64')
            || (constraint.value === 255 && constraint.check === 'less_than' && nativeType === 'u8')))
            break
          const operator = `${constraint.check === 'greater_than' ? '>' : '<'}${constraint.inclusive ? '=' : ''}`
          require(`*${value} ${operator} ${constraint.value}${this.type(schema, 'Number') === 'f64' ? '_f64' : ''}`, 'number out of range')
          break
        }
        case 'min_length':
        case 'max_length': {
          const length = def.type === 'string' ? `${value}.encode_utf16().count()` : `${value}.len()`
          require(`${length} ${constraint.check === 'min_length' ? '>=' : '<='} ${constraint.check === 'min_length' ? constraint.minimum : constraint.maximum}`, 'invalid length')
          break
        }
        case 'string_format': {
          if (constraint.format !== 'regex')
            throw new Error(`Unsupported string format ${constraint.format}`)
          const pattern = (constraint as z.core.$ZodCheckRegexDef).pattern
          if (pattern.flags && pattern.flags !== 'u')
            throw new Error(`Unsupported regex flags: ${pattern.flags}`)
          let name = this.patterns.get(pattern.source)
          if (!name) {
            name = `PATTERN_${this.patterns.size}`
            this.patterns.set(pattern.source, name)
          }
          require(`${name}.is_match(${expression})`, 'invalid string format')
          break
        }
        case 'custom':
          if (z.globalRegistry.get(schema)?.uniqueItems !== true || def.type !== 'array'
            || def.checks?.filter(item => item._zod.def.check === 'custom').length !== 1)
            throw new Error('Custom Zod refinements need a supported declarative constraint')
          lines.push(`let mut seen = std::collections::HashSet::new();`)
          require(`${value}.iter().all(|item| seen.insert(item))`, 'array items must be unique')
          break
        default: throw new Error(`Unsupported Zod check: ${constraint.check}`)
      }
    }
    return lines.filter(line => line.trim()).join('\n')
  }

  /**
   * A union of objects tagged by a literal `type` field, as an internally
   * tagged serde enum with one struct variant per tag. Field types are named
   * after `prefix` and the tag.
   */
  taggedEnum(
    schema: z.core.$ZodType,
    name: string,
    options: { prefix?: string, derive?: string } = {},
  ): string {
    const variants = unionOptions(schema).map(variant => {
      const tag = (variant.shape.type as z.ZodLiteral<string>).value
      const fields = this.fields(variant, `${options.prefix ?? ''}${rustPascal(tag)}`, new Set(['type'])).replaceAll('pub ', '')
      return `#[serde(rename = ${rustString(tag)})]\n${rustPascal(tag)} {\n${fields}\n},`
    })
    return `#[derive(${options.derive ?? 'Debug, Clone, serde::Serialize, serde::Deserialize'})]
    #[serde(tag = "type", deny_unknown_fields)]
    pub enum ${name} { ${variants.join('\n')} }`
  }

  finish(): string {
    const patterns = [...this.patterns].map(([pattern, name]) =>
      `static ${name}: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(${rustString(pattern.replaceAll('\\0', '\\x00'))}).expect("generated contract regex"));`)
    return [...this.declarations.values(), ...this.deserializers, ...patterns].join('\n\n')
  }
}
