import { z } from 'zod'

const requestIdSchema = z.int().positive()
const bytesSchema: z.ZodType<Uint8Array> = z.instanceof(Uint8Array)
const ioReplyValueSchema = bytesSchema.nullable()
const runCommandSchema = z.strictObject({
  type: z.literal('run'),
  path: z.string().min(1),
  args: z.record(z.string(), z.unknown()),
  cwd: z.string().min(1),
  env: z.record(z.string(), z.string()),
})
const inputRequestSchema = z.strictObject({
  type: z.literal('input'),
  id: requestIdSchema,
})
const outputRequestSchema = z.strictObject({
  type: z.literal('output'),
  id: requestIdSchema,
  stream: z.enum(['stdout', 'stderr']),
  bytes: bytesSchema,
})
export const workerMessageSchema = z.discriminatedUnion('type', [
  inputRequestSchema,
  outputRequestSchema,
  z.strictObject({
    type: z.literal('exit'),
    exitCode: z.int().min(0).max(255),
  }),
  z.strictObject({ type: z.literal('error'), message: z.string() }),
])
export const runnerMessageSchema = z.union([
  runCommandSchema,
  z.strictObject({
    type: z.literal('reply'),
    id: requestIdSchema,
    value: ioReplyValueSchema,
  }),
  z.strictObject({
    type: z.literal('reply'),
    id: requestIdSchema,
    error: z.string(),
  }),
])
export type RunCommand = z.infer<typeof runCommandSchema>
export type InputRequest = z.infer<typeof inputRequestSchema>
export type OutputRequest = z.infer<typeof outputRequestSchema>
export type IORequest = InputRequest | OutputRequest
export type IOReplyValue = z.infer<typeof ioReplyValueSchema>
export type WorkerMessage = z.infer<typeof workerMessageSchema>
export type RunnerMessage = z.infer<typeof runnerMessageSchema>
