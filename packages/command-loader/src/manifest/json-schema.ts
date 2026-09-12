import { z } from 'zod'

const annotations = {
  $schema: z.string().optional(),
  title: z.string().optional(),
  description: z.string().optional(),
}

function nullableType<T extends string>(type: T) {
  return z.union([
    z.literal(type),
    z.tuple([z.literal(type), z.literal('null')]),
    z.tuple([z.literal('null'), z.literal(type)]),
  ])
}

const scalarVariants = [
  { type: z.literal('string'), value: z.string() },
  { type: z.literal('number'), value: z.number() },
  { type: z.literal('integer'), value: z.number().int() },
  { type: z.literal('boolean'), value: z.boolean() },
  { type: z.literal('null'), value: z.null() },
]
const scalarSchemas = scalarVariants.flatMap(({ type, value }) => [
  z.strictObject({ ...annotations, type, const: value }),
  z.strictObject({ ...annotations, type, enum: z.array(value).min(1) }),
])
const length = z.number().int().nonnegative()

/** The supported manifest subset; unknown validation keywords must not disappear. */
export const commandJsonSchema: z.ZodType<Record<string, unknown>> = z.lazy(() =>
  z.union([
    z.strictObject(annotations),
    z.strictObject({
      ...annotations,
      type: nullableType('string'),
      minLength: length.optional(),
      maxLength: length.optional(),
    }),
    z.strictObject({
      ...annotations,
      type: z.union([nullableType('number'), nullableType('integer')]),
      minimum: z.number().optional(),
      maximum: z.number().optional(),
      exclusiveMinimum: z.number().optional(),
      exclusiveMaximum: z.number().optional(),
      multipleOf: z.number().positive().optional(),
    }),
    z.strictObject({ ...annotations, type: nullableType('boolean') }),
    z.strictObject({ ...annotations, type: z.literal('null') }),
    ...scalarSchemas,
    z.strictObject({
      ...annotations,
      type: nullableType('array'),
      items: commandJsonSchema,
      minItems: length.optional(),
      maxItems: length.optional(),
    }),
    z.strictObject({
      ...annotations,
      type: nullableType('object'),
      properties: z.record(z.string(), commandJsonSchema),
      required: z.array(z.string()).optional(),
      additionalProperties: z.union([z.boolean(), commandJsonSchema]).optional(),
    }),
    z.strictObject({
      ...annotations,
      anyOf: z.array(commandJsonSchema).min(2),
    }),
  ])
)
