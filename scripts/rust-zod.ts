import type { z } from 'zod'

export function rustPascal(value: string): string {
  return value.replace(/(^|[_-])([a-z])/g, (_, _prefix, letter) => letter.toUpperCase())
}

function rustField(value: string): string {
  const name = value.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replaceAll('-', '_').toLowerCase()
  return ['type', 'ref', 'self', 'match', 'loop', 'move', 'mod', 'fn'].includes(name) ? `r#${name}` : name
}

/** Generates strict serde values from the same Zod declarations used by TypeScript. */
export class RustZodTypes {
  readonly declarations = new Map<string, string>()
  private readonly names = new Map<z.ZodType, string>()

  constructor(private readonly overrides = new Map<z.ZodType, string>()) {}

  fields(schema: z.ZodObject): string {
    return Object.entries(schema.shape).map(([field, child]) => {
      const optional = child._zod.def.type === 'optional'
      const actual = optional ? (child as z.ZodOptional).unwrap() : child
      const type = this.type(actual, `${this.names.get(schema)}${rustPascal(field)}`)
      const serde = `#[serde(rename = ${JSON.stringify(field)}${optional ? ', default, skip_serializing_if = "Option::is_none"' : ''})]`
      return `${serde}\npub ${rustField(field)}: ${optional ? `Option<${type}>` : type},`
    }).join('\n')
  }

  type(schema: z.ZodType, name: string): string {
    const known = this.overrides.get(schema) ?? this.names.get(schema)
    if (known)
      return known
    const def = schema._zod.def as Record<string, any>
    const derive = '#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]'
    switch (def.type) {
      case 'string':
      case 'enum': return 'String'
      case 'boolean': return 'bool'
      case 'number': return schema.safeParse(0.5).success ? 'f64' : schema.safeParse(-1).success ? 'i64' : 'u64'
      case 'unknown': return 'serde_json::Value'
      case 'null': return '()'
      case 'literal': return typeof def.values[0] === 'number' ? 'u64' : typeof def.values[0] === 'boolean' ? 'bool' : 'String'
      case 'optional':
      case 'nullable': return `Option<${this.type(def.innerType, name)}>`
      case 'array': return `Vec<${this.type(def.element, `${name}Item`)}>`
      case 'record': return `std::collections::BTreeMap<String, ${this.type(def.valueType, `${name}Value`)}>`
      case 'lazy': {
        this.names.set(schema, name)
        return this.type(def.getter(), name)
      }
      case 'object': {
        this.names.set(schema, name)
        const fields = this.fields(schema as z.ZodObject)
        this.declarations.set(name, `${derive}\n#[serde(deny_unknown_fields)]\npub struct ${name} {\n${fields}\n}`)
        return name
      }
      case 'union': {
        this.names.set(schema, name)
        const variants = def.options.map((child: z.ZodType, index: number) => `Variant${index}(Box<${this.type(child, `${name}Variant${index}`)}>),`).join('\n')
        this.declarations.set(name, `${derive}\n#[serde(untagged)]\npub enum ${name} {\n${variants}\n}`)
        return name
      }
      default: throw new Error(`Unsupported Zod Rust value: ${def.type}`)
    }
  }
}
