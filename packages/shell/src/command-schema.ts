import { z } from 'zod'
import { errorMessage } from '@demicodes/utils'

const cliBooleanSchema = z.stringbool({
  truthy: ['true'],
  falsy: ['false'],
  case: 'sensitive',
})

/** Unwrap only representations supported by the command contract. */
export function commandValueSchema(schema: z.core.$ZodType): z.core.$ZodType {
  if (schema instanceof z.ZodOptional || schema instanceof z.ZodNullable) {
    return commandValueSchema(schema.unwrap())
  }
  if (schema instanceof z.ZodUnion) {
    const values = schema.options.filter((option) => !(option instanceof z.ZodNull))
    if (schema.options.length === 2 && values.length === 1) {
      return commandValueSchema(values[0]!)
    }
  }
  return schema
}

export function commandScalarKind(schema: z.core.$ZodType): 'string' | 'number' | 'boolean' | null {
  const value = commandValueSchema(schema)
  if (value instanceof z.ZodString) {
    return 'string'
  }
  if (value instanceof z.ZodNumber) {
    return 'number'
  }
  if (value instanceof z.ZodBoolean) {
    return 'boolean'
  }
  const literals = commandChoices(value) ?? []
  const kind = typeof literals[0]
  if (literals.length > 0 && literals.every((entry) => typeof entry === kind)
    && (kind === 'string' || kind === 'number' || kind === 'boolean')) {
    return kind
  }
  return null
}

/** JSON Schema reconstruction represents numeric enums as unions of literals. */
export function commandChoices(schema: z.core.$ZodType): readonly unknown[] | null {
  const value = commandValueSchema(schema)
  if (value instanceof z.ZodLiteral) {
    return [...value.values]
  }
  if (value instanceof z.ZodEnum) {
    return value.options
  }
  if (value instanceof z.ZodUnion
    && value.options.every((option) => option instanceof z.ZodLiteral)) {
    return value.options.flatMap((option) => [...option.values])
  }
  return null
}

/** CLI adaptation changes representations; the declaring schema validates values. */
export function decodeCommandValue(schema: z.core.$ZodType, value: unknown): unknown {
  if (value === undefined || value === null) {
    return value
  }
  const inner = commandValueSchema(schema)
  if (inner instanceof z.ZodArray) {
    const values = Array.isArray(value) ? value : [value]
    return values.map((entry) => decodeCommandValue(inner.element, entry))
  }
  if (typeof value !== 'string') {
    return value
  }
  switch (commandScalarKind(inner)) {
    case 'number':
      return value.trim() === '' ? value : Number(value)
    case 'boolean': {
      const result = cliBooleanSchema.safeParse(value)
      return result.success ? result.data : value
    }
    default:
      return value
  }
}

/** Check both CLI representability and the behavior retained by JSON Schema. */
export function validateCommandFieldSchema(schema: z.core.$ZodType, path: string): void {
  const value = commandValueSchema(schema)
  if (value instanceof z.ZodArray) {
    if (commandScalarKind(value.element) === null || z.safeParse(value.element, undefined).success) {
      throw new Error(`CommandRegistry: "${path}" requires scalar array elements`)
    }
  } else if (commandScalarKind(value) === null) {
    throw new Error(`CommandRegistry: "${path}" requires a scalar or scalar array schema`)
  }
  validateCommandSerialization(schema, path)
}

export function validateCommandSerialization(schema: z.core.$ZodType, path: string): void {
  try {
    z.toJSONSchema(schema, {
      cycles: 'throw',
      override: ({ zodSchema }) => {
        const def = zodSchema._zod.def
        if (![
          'string', 'number', 'boolean', 'null', 'literal', 'enum', 'array',
          'object', 'unknown', 'any', 'never', 'optional', 'nullable', 'union',
        ].includes(def.type)) {
          throw new Error(`unsupported schema type ${def.type}`)
        }
        if ('coerce' in def && def.coerce) {
          throw new Error('coercion belongs to argv decoding')
        }
        if ('format' in def && def.format !== 'safeint') {
          throw new Error('formatted scalar schemas are not supported')
        }
        for (const check of def.checks ?? []) {
          const constraint = check._zod.def
          if (constraint.check === 'number_format'
            && 'format' in constraint && constraint.format === 'safeint') {
            continue
          }
          if (![
            'min_length', 'max_length', 'length_equals',
            'greater_than', 'less_than', 'multiple_of',
          ].includes(constraint.check)) {
            throw new Error(`unsupported schema check ${constraint.check}`)
          }
        }
      },
    })
  } catch (error) {
    throw new Error(`CommandRegistry: "${path}" cannot cross a manifest boundary: ${errorMessage(error)}`, {
      cause: error,
    })
  }
}
