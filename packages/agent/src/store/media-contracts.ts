import { z } from 'zod'
import {
  base64SourceSchema,
  binarySourceSchema,
  createToolContentSchema,
  createUserContentSchema,
  documentSourceSchema,
  urlSourceSchema,
} from '../protocol/schemas'
import { createBlockSchema } from '../protocol/block-schemas'
import { createServerFrameSchema } from '../protocol/server-schemas'

import { blobKeySchema } from './blob'
const refSourceSchema = z.strictObject({
  type: z.literal('ref'),
  ref: blobKeySchema,
  mediaType: z.string(),
})
const documentRefSchema = refSourceSchema.extend({ fileName: z.string().min(1) })
const toolRefSchema = z.strictObject({ ref: blobKeySchema, mediaType: z.string() })

export const storedUserContentSchema = createUserContentSchema(
  z.union([refSourceSchema, urlSourceSchema]),
  documentRefSchema,
)
export const storedToolContentSchema = createToolContentSchema(toolRefSchema)
export const storedBlockSchema = createBlockSchema(storedUserContentSchema, storedToolContentSchema)

export const displayedUserContentSchema = createUserContentSchema(
  z.union([refSourceSchema, urlSourceSchema, binarySourceSchema]),
  z.union([documentRefSchema, documentSourceSchema]),
)
export const displayedToolContentSchema = createToolContentSchema(
  z.union([toolRefSchema, base64SourceSchema]),
)
export const displayedBlockSchema = createBlockSchema(
  displayedUserContentSchema,
  displayedToolContentSchema,
)

export type StoredUserContent = z.infer<typeof storedUserContentSchema>
export type StoredToolContent = z.infer<typeof storedToolContentSchema>
export type StoredBlock = z.infer<typeof storedBlockSchema>
export type DisplayedUserContent = z.infer<typeof displayedUserContentSchema>
export type DisplayedToolContent = z.infer<typeof displayedToolContentSchema>
export type DisplayedBlock = z.infer<typeof displayedBlockSchema>

export const displayedServerFrameSchema = createServerFrameSchema(displayedBlockSchema)
