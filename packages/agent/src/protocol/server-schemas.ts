import { z } from 'zod'
import { blockSchema, diagnosticsSchema } from './block-schemas'
import {
  bytesSchema,
  editResultSchema,
  metadataSchema,
  pendingSteersFrameSchema,
  queuedMessageSchema,
  sessionPhaseSchema,
  toolResultContentBlockSchema,
} from './schemas'

const blockPath = z.tuple([z.literal('blocks'), z.number().int().nonnegative()])
export function createTranscriptPatchSchema<S extends z.ZodType>(blockSchema: S) {
  return z.discriminatedUnion('op', [
    z.object({
      op: z.literal('add'),
      path: blockPath,
      value: blockSchema,
    }),
    z.object({
      op: z.literal('remove'),
      path: blockPath,
    }),
    z.object({
      op: z.literal('replace_block'),
      path: blockPath,
      value: blockSchema,
    }),
    z.object({
      op: z.literal('append_text'),
      path: blockPath,
      delta: z.string(),
    }),
    z.object({
      op: z.literal('replace'),
      path: z.tuple([z.literal('blocks')]),
      value: z.array(blockSchema),
    }),
  ])
}

const shellStream = z.object({
  path: z.string().optional(),
  offset: z.number().int().nonnegative(),
  delta: z.string(),
  tail: z.string(),
  bytes: z.number().int().nonnegative(),
  truncated: z.boolean(),
})
const shellOutput = z.object({
  path: z.string().optional(),
  offset: z.number().int().nonnegative(),
  text: z.string(),
  tail: z.string(),
  chunks: z.array(
    z.object({
      stream: z.enum(['stdout', 'stderr']),
      text: z.string(),
    }),
  ),
  bytes: z.number().int().nonnegative(),
  truncated: z.boolean(),
})
const shellMeta = {
  shellId: z.string(),
  commandId: z.string(),
  outputDir: z.string().optional(),
  stdout: shellStream,
  stderr: shellStream,
  output: shellOutput,
  runningMs: z.number().nonnegative(),
  idleMs: z.number().nonnegative(),
}
export const shellStatusSchema = z.discriminatedUnion('status', [
  z.object({
    ...shellMeta,
    status: z.literal('running'),
    runningHint: z.string().optional(),
  }),
  z.object({
    ...shellMeta,
    status: z.literal('aborted'),
  }),
  z.object({
    ...shellMeta,
    status: z.literal('exited'),
    exitCode: z.number().int(),
    binaryStdout: z
      .object({
        data: bytesSchema,
        truncated: z.boolean(),
        totalBytes: z.number().int().nonnegative(),
        limitBytes: z.number().int().nonnegative(),
      })
      .optional(),
  }),
])
export function createServerFrameSchema<S extends z.ZodType>(blockSchema: S) {
  const patchSchema = createTranscriptPatchSchema(blockSchema)
  return z.discriminatedUnion('type', [
    z.object({ type: z.literal('opened') }),
    editResultSchema,
    z.object({ type: z.literal('closed') }),
    z.object({
      type: z.literal('rejected'),
      command: z.string(),
      reason: z.string(),
    }),
    z.object({
      type: z.literal('transcript_reset'),
      blocks: z.array(blockSchema),
      epoch: z.string().min(1),
      revision: z.number().int().nonnegative(),
    }),
    z.object({
      type: z.literal('transcript_patch'),
      patches: z.array(patchSchema),
      revision: z.number().int().nonnegative(),
    }),
    z.object({
      type: z.literal('phase'),
      phase: sessionPhaseSchema,
    }),
    z.object({ type: z.literal('queue'), queue: z.array(queuedMessageSchema) }),
    pendingSteersFrameSchema,
    z.discriminatedUnion('status', [
      z.object({
        type: z.literal('steer_result'),
        steerId: z.string(),
        status: z.literal('accepted'),
      }),
      z.object({
        type: z.literal('steer_result'),
        steerId: z.string(),
        status: z.literal('rejected'),
        reason: z.string(),
      }),
    ]),
    z.object({
      type: z.literal('abort_result'),
      result: z.object({
        aborted: z.boolean(),
        target: z
          .enum([
            'active_provider_stream',
            'active_tool',
            'active_compaction',
            'active_turn',
            'queued_action',
            'queued_message',
            'pending_yield_wakeup',
          ])
          .nullable(),
        canAbortAgain: z.boolean(),
      }),
    }),
    z.object({
      type: z.literal('tool_progress'),
      toolUseId: z.string(),
      output: z.array(toolResultContentBlockSchema),
    }),
    z.object({
      type: z.literal('shell_output'),
      shellId: z.string(),
      commandId: z.string(),
      status: shellStatusSchema,
    }),
    z.object({
      type: z.literal('shell_write_result'),
      commandId: z.string(),
      output: z.array(toolResultContentBlockSchema),
    }),
    z.object({
      type: z.literal('retry_scheduled'),
      attempt: z.number().int().nonnegative(),
      delayMs: z.number().nonnegative(),
      code: z.string().nullable(),
      diagnostics: diagnosticsSchema.optional(),
    }),
    z.object({
      type: z.literal('error'),
      message: z.string(),
      code: z.string().optional(),
      diagnostics: diagnosticsSchema.optional(),
    }),
    z.object({
      type: z.literal('subagent'),
      event: z.enum(['started', 'closed']),
      job: z.object({
        subagentId: z.string(),
        parentSessionId: z.string(),
        description: z.string(),
        profile: z.string().nullable(),
        phase: z.enum(['running', 'completed', 'aborted', 'error']),
        startedAt: z.string(),
        endedAt: z.string().nullable(),
        metadata: metadataSchema.nullable(),
        result: z.string().optional(),
      }),
    }),
    z.object({
      type: z.literal('subagent_transcript_reset'),
      subagentId: z.string(),
      blocks: z.array(blockSchema),
      revision: z.number().int().nonnegative(),
    }),
    z.object({
      type: z.literal('subagent_transcript_patch'),
      subagentId: z.string(),
      patches: z.array(patchSchema),
      revision: z.number().int().nonnegative(),
    }),
  ])
}

export const serverFrameSchema = createServerFrameSchema(blockSchema)
