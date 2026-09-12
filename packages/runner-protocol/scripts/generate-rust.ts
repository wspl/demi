import { mkdir, writeFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { z } from 'zod'
import {
  backendToRunnerMessageSchema,
  runnerToBackendMessageSchema,
} from '../src/schemas'
import { JOB_VIEW_BYTES, RUNNER_PROTOCOL_VERSION } from '../src/messages'

// The Zod wire contract owns fields and constraints. Generated Rust decodes the
// MessagePack types, then checks all remaining constraints against the same schema.
const directory = resolve(import.meta.dir, '../rust')
const declarations = new Map<string, string>()
const objectTypes = new Map<z.ZodType, string>()
const derive = '#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]'

function pascal(value: string): string {
  return value.replace(/(^|[_-])([a-z])/g, (_, _prefix, letter) => letter.toUpperCase())
}

function snake(value: string): string {
  return value.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replaceAll('-', '_').toLowerCase()
}

function rustName(value: string): string {
  const name = snake(value)
  return ['type', 'ref', 'self', 'match', 'loop', 'move', 'mod', 'fn'].includes(name)
    ? `r#${name}` : name
}

function definition(schema: z.ZodType): Record<string, any> {
  return schema._zod.def
}

function fields(schema: z.ZodObject, name: string, skip: Set<string> = new Set()): string {
  return Object.entries(schema.shape).filter(([field]) => !skip.has(field)).map(([field, child]) => {
    const optional = definition(child).type === 'optional'
    const actual = optional ? (child as z.ZodOptional).unwrap() : child
    const type = rustType(actual, `${name}${pascal(field)}`)
    return [
      `#[serde(rename = ${JSON.stringify(field)}${optional ? ', default, skip_serializing_if = "Option::is_none"' : ''})]`,
      `pub ${rustName(field)}: ${optional ? `Option<${type}>` : type},`,
    ].join('\n')
  }).join('\n')
}

function rustType(schema: z.ZodType, name: string): string {
  const def = definition(schema)
  switch (def.type) {
    case 'string': return 'String'
    case 'boolean': return 'bool'
    case 'number': return schema.safeParse(0.5).success ? 'f64'
      : schema.safeParse(-1).success ? 'i64' : 'u64'
    case 'custom': return 'crate::WireBytes'
    case 'date': return 'crate::Timestamp'
    case 'unknown': return 'serde_json::Value'
    case 'null': return '()'
    case 'optional':
    case 'nullable': return `Option<${rustType(def.innerType, name)}>`
    case 'array': return `Vec<${rustType(def.element, `${name}Item`)}>`
    case 'record': return `std::collections::BTreeMap<String, ${rustType(def.valueType, `${name}Value`)}>`
    case 'object': {
      const existing = objectTypes.get(schema)
      if (existing)
        return existing
      objectTypes.set(schema, name)
      const body = fields(schema as z.ZodObject, name)
      declarations.set(name, `${derive}\n#[serde(deny_unknown_fields)]\npub struct ${name} {\n${body}\n}`)
      return name
    }
    case 'enum': return 'String'
    case 'literal': {
      const value = (schema as z.ZodLiteral).value
      return typeof value === 'number' ? 'u64' : typeof value === 'boolean' ? 'bool' : 'String'
    }
    case 'union': {
      const variants = def.options.map((child: z.ZodType, index: number) =>
        `Variant${index}(${rustType(child, `${name}Variant${index}`)}),`).join('\n')
      declarations.set(name, `${derive}\n#[serde(untagged)]\npub enum ${name} {\n${variants}\n}`)
      return name
    }
    default: throw new Error(`Unsupported Zod wire shape: ${def.type}`)
  }
}

function flatten(schema: z.ZodType): z.ZodObject[] {
  const def = definition(schema)
  return def.type === 'union' ? def.options.flatMap(flatten) : [schema as z.ZodObject]
}

export function generateRustValues(values: Record<string, z.ZodType>): string {
  declarations.clear()
  objectTypes.clear()
  for (const [name, schema] of Object.entries(values)) {
    const type = rustType(schema, name)
    if (type !== name)
      declarations.set(name, `pub type ${name} = ${type};`)
  }
  return ['// Generated from Zod contracts. Do not edit.', ...declarations.values(), ''].join('\n\n')
}

async function main(): Promise<void> {
  const inboundVariants = flatten(backendToRunnerMessageSchema).map(schema => {
    const type = (schema.shape.type as z.ZodLiteral<string>).value
    const name = pascal(type)
    // Enum variant fields cannot be public; the enum already exposes them.
    const body = fields(schema, name, new Set(['type'])).replaceAll('pub ', '')
    return `#[serde(rename = ${JSON.stringify(type)})]\n${name} {\n${body}\n},`
  })

  const outgoing = flatten(runnerToBackendMessageSchema).map(schema => {
    const literals = Object.entries(schema.shape).filter(([, child]) => definition(child).type === 'literal')
    const type = (schema.shape.type as z.ZodLiteral<string>).value
    const operation = schema.shape.op ? (schema.shape.op as z.ZodLiteral<string>).value : ''
    const name = `${pascal(type)}${pascal(operation)}`
    const constantNames = new Set(literals.map(([key]) => key))
    const variableFields = Object.entries(schema.shape).filter(([key]) => !constantNames.has(key))
    const args = variableFields.map(([key, value]) => `${rustName(key)}: ${rustType(value, `${name}${pascal(key)}`)}`)
    const constants = literals.map(([key, value]) => `${rustName(key)}: ${JSON.stringify((value as z.ZodLiteral<string>).value)}`)
    const structFields = literals.map(([key]) => `#[serde(rename = ${JSON.stringify(key)})]\n${rustName(key)}: &'static str,`)
    const body = fields(schema, name, constantNames).replaceAll('pub ', '')
    return `#[allow(clippy::too_many_arguments)] // Generated message fields mirror the wire contract.
  pub fn ${snake(name)}(${args.join(', ')}) -> Result<crate::Outbound, crate::WireError> {
  #[derive(serde::Serialize)]
  struct Message {
  ${structFields.join('\n')}
  ${body}
  }
  crate::encode(&Message { ${[...constants, ...variableFields.map(([key]) => rustName(key))].join(', ')} })
  }`
  })

  const schema = z.toJSONSchema(backendToRunnerMessageSchema, {
    unrepresentable: 'any',
    override(context) {
      const type = context.zodSchema._zod.def.type
      if (type === 'custom') {
        Object.assign(context.jsonSchema, { type: 'array', items: { type: 'integer', minimum: 0, maximum: 255 } })
      } else if (type === 'date') {
        Object.assign(context.jsonSchema, { type: 'integer' })
      }
    },
  })

  await mkdir(directory, { recursive: true })
  await writeFile(resolve(directory, 'generated.rs'), [
    '// Generated by scripts/generate-rust.ts. Edit src/schemas.ts instead.',
    `pub const VERSION: u32 = ${RUNNER_PROTOCOL_VERSION};`,
    `pub const JOB_VIEW_BYTES: usize = ${JOB_VIEW_BYTES};`,
    `${derive}\n#[serde(tag = "type", deny_unknown_fields)]\npub enum Inbound {\n${inboundVariants.join('\n')}\n}`,
    ...declarations.values(),
    ...outgoing,
    '',
  ].join('\n\n'))
  await writeFile(resolve(directory, 'inbound.schema.json'), `${JSON.stringify(schema, null, 2)}\n`)

}

if (import.meta.main)
  await main()
