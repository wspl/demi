import { expect, test } from 'bun:test'
import { z } from 'zod'
import { RustZodTypes } from '../../rust-zod'

// A new constraint must stop the build instead of disappearing from native validation.
test('native contract generation rejects unsupported constraints and object policies', () => {
  for (const schema of [
    z.object({ field: z.string() }),
    z.strictObject({ field: z.string().email() }),
    z.strictObject({ field: z.number().multipleOf(3) }),
    z.strictObject({ field: z.string().refine(value => value.startsWith('a')) }),
    z.strictObject({ field: z.string().transform(value => value.toUpperCase()) }),
    z.strictObject({ field: z.string().default('implicit') }),
  ]) {
    expect(() => new RustZodTypes().type(schema, 'Contract')).toThrow()
  }
})
