import { z } from 'zod'
import type { Block } from '@demicodes/core'
import {
  modelSelectionSchema,
  editResultSchema,
  type ServerFrame,
  type ClientFrame,
} from '@demicodes/web-ui/transport/protocol'

const text = z.object({
  type: z.literal('text'),
  text: z.string(),
})
const refSource = z.object({
  type: z.literal('ref'),
  ref: z.string().min(1),
  mediaType: z.string(),
  fileName: z.string().optional(),
})
const urlSource = z.object({
  type: z.literal('url'),
  url: z.string(),
})
const binarySource = z.object({
  type: z.literal('binary'),
  data: z.instanceof(Uint8Array),
  mediaType: z.string(),
})
export const displayedUserContentSchema = z.discriminatedUnion('type', [
  text,
  z.object({
    type: z.literal('reference'),
    reference: z.string(),
  }),
  z.object({
    type: z.literal('image'),
    source: z.union([refSource, urlSource, binarySource]),
  }),
  z.object({
    type: z.literal('video'),
    source: z.union([refSource, urlSource, binarySource]),
  }),
  z.object({
    type: z.literal('document'),
    source: z.union([
      refSource,
      z.object({
        data: z.instanceof(Uint8Array),
        mediaType: z.string(),
        fileName: z.string(),
      }),
    ]),
  }),
])
const toolSource = z.union([
  refSource,
  z.object({
    data: z.string(),
    mediaType: z.string(),
  }),
])
const toolContent = z.discriminatedUnion('type', [
  text,
  z.object({
    type: z.literal('image'),
    source: toolSource,
  }),
  z.object({
    type: z.literal('video'),
    source: toolSource,
  }),
])
const meta = {
  id: z.string(),
  createdAt: z.string(),
  model: modelSelectionSchema,
}
const diagnostics = z.object({
  source: z.enum(['http', 'stream', 'transport', 'unknown']),
  clientRequestId: z.string().optional(),
  providerRequestId: z.string().optional(),
  providerResponseId: z.string().optional(),
  providerCode: z.string().optional(),
  httpStatus: z.number().optional(),
})
export const usageSchema = z.object({
  inputTokens: z.number(),
  outputTokens: z.number(),
  cacheReadTokens: z.number(),
  cacheWriteTokens: z.number(),
})
const persistedBlockSchema = z.discriminatedUnion('type', [
  z.object({
    ...meta,
    type: z.literal('user'),
    turnId: z.string(),
    content: z.array(displayedUserContentSchema),
    preamble: z.string().nullable(),
    resolvedContent: z.array(displayedUserContentSchema).optional(),
    hidden: z.boolean().optional(),
  }),
  z.object({
    ...meta,
    type: z.literal('resume'),
    turnId: z.string(),
  }),
  z.object({
    ...meta,
    type: z.literal('steer'),
    turnId: z.string(),
    content: z.array(displayedUserContentSchema),
    hidden: z.boolean().optional(),
  }),
  z.object({
    ...meta,
    type: z.literal('thinking'),
    text: z.string(),
    signature: z.string().nullable(),
  }),
  z.object({
    ...meta,
    type: z.literal('redacted_thinking'),
    data: z.string(),
  }),
  z.object({
    ...meta,
    type: z.literal('text'),
    text: z.string(),
    forkable: z.literal(true).optional(),
  }),
  z.object({
    ...meta,
    type: z.literal('tool_call'),
    toolUseId: z.string(),
    toolName: z.string(),
    input: z.string(),
    status: z.enum(['executing', 'completed', 'error']),
    streamingOutput: z.array(toolContent),
    output: z.array(toolContent),
    view: z.unknown(),
  }),
  z.object({
    ...meta,
    type: z.literal('response'),
    usage: usageSchema,
  }),
  z.object({
    ...meta,
    type: z.literal('error'),
    message: z.string(),
    code: z.string().nullable(),
    diagnostics: diagnostics.optional(),
  }),
  z.object({
    ...meta,
    type: z.literal('abort'),
    isResumed: z.boolean(),
  }),
  z.object({
    ...meta,
    type: z.literal('compaction_boundary'),
    summary: z.string(),
    summaryTokens: z.number(),
  }),
  z.object({
    ...meta,
    type: z.literal('compaction_marker'),
    boundaryId: z.string(),
    compactedTokens: z.number(),
  }),
  z.object({
    type: z.literal('extension_state_snapshot'),
    id: z.string(),
    createdAt: z.string(),
    extensionName: z.string(),
    state: z.unknown(),
  }),
])

// The backend externalizes media to blob references. The shared transcript
// renderers accept that storage form; it is validated here before the core view
// contract is used. It must never be replayed directly as provider input.
export const blockSchema = persistedBlockSchema.transform(
  (block) => block as unknown as Block,
)
export const transcriptSchema = z.object({
  blocks: z.array(blockSchema),
  subagents: z.array(
    z.object({
      id: z.string(),
      name: z.string(),
      phase: z.enum(['running', 'completed', 'aborted', 'error']),
      startedAt: z.string(),
      endedAt: z.string().nullable(),
      blocks: z.array(blockSchema),
    }),
  ),
})
const blockPath = z.tuple([z.literal('blocks'), z.number().int().nonnegative()])
const patchSchema = z.discriminatedUnion('op', [
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
const shellStream = z.object({
  path: z.string().optional(),
  offset: z.number(),
  delta: z.string(),
  tail: z.string(),
  bytes: z.number(),
  truncated: z.boolean(),
})
const shellOutput = z.object({
  path: z.string().optional(),
  offset: z.number(),
  text: z.string(),
  tail: z.string(),
  chunks: z.array(
    z.object({
      stream: z.enum(['stdout', 'stderr']),
      text: z.string(),
    }),
  ),
  bytes: z.number(),
  truncated: z.boolean(),
})
const shellMeta = {
  shellId: z.string(),
  commandId: z.string(),
  outputDir: z.string().optional(),
  stdout: shellStream,
  stderr: shellStream,
  output: shellOutput,
  runningMs: z.number(),
  idleMs: z.number(),
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
    exitCode: z.number(),
    binaryStdout: z
      .object({
        data: z.instanceof(Uint8Array),
        truncated: z.boolean(),
        totalBytes: z.number(),
        limitBytes: z.number(),
      })
      .optional(),
  }),
])
const serverFrameSchema = z.discriminatedUnion('type', [
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
    phase: z.enum(['idle', 'running', 'compacting']),
  }),
  z.object({
    type: z.literal('queue'),
    queue: z.array(
      z.object({
        id: z.string(),
        text: z.string(),
        content: z.array(displayedUserContentSchema),
      }),
    ),
  }),
  z.object({
    type: z.literal('pending_steers'),
    pendingSteers: z.array(
      z.object({
        id: z.string(),
        turnId: z.string(),
        model: modelSelectionSchema,
        content: z.array(displayedUserContentSchema),
      }),
    ),
  }),
  z.object({
    type: z.literal('steer_result'),
    steerId: z.string(),
    status: z.enum(['accepted', 'rejected']),
    reason: z.string().optional(),
  }),
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
    output: z.array(toolContent),
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
    output: z.array(toolContent),
  }),
  z.object({
    type: z.literal('retry_scheduled'),
    attempt: z.number(),
    delayMs: z.number(),
    code: z.string().nullable(),
    diagnostics: diagnostics.optional(),
  }),
  z.object({
    type: z.literal('error'),
    message: z.string(),
    code: z.string().optional(),
    diagnostics: diagnostics.optional(),
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
      metadata: z.record(z.string(), z.unknown()).nullable(),
      result: z.string().optional(),
    }),
  }),
  z.object({
    type: z.literal('subagent_transcript_reset'),
    subagentId: z.string(),
    blocks: z.array(blockSchema),
    revision: z.number(),
  }),
  z.object({
    type: z.literal('subagent_transcript_patch'),
    subagentId: z.string(),
    patches: z.array(patchSchema),
    revision: z.number(),
  }),
])

export function decodeServerFrame(value: unknown): ServerFrame {
  return serverFrameSchema.parse(value) as unknown as ServerFrame
}

const outgoingReferenceSchema = z.discriminatedUnion('type', [
  z.object({
    type: z.literal('remote_file'),
    deviceId: z.string(),
    path: z.string(),
  }),
  z.object({
    type: z.enum(['image', 'video', 'document']),
    source: z.object({
      type: z.literal('ref'),
      ref: z.string(),
      fileName: z.string().optional(),
    }),
  }),
])
export type OutgoingReference = z.infer<typeof outgoingReferenceSchema>
const referencePrefix = 'demi-upload:'
export function contentReference(value: OutgoingReference) {
  return {
    type: 'reference' as const,
    reference: referencePrefix + JSON.stringify(value),
  }
}

/** Keep the runtime's core content contract while adapting product wire refs. */
export function encodeClientFrame(frame: ClientFrame): unknown {
  if (!('content' in frame) || !Array.isArray(frame.content)) {
    return frame
  }
  return {
    ...frame,
    content: frame.content.map((block) =>
      block.type === 'reference' && block.reference.startsWith(referencePrefix)
        ? outgoingReferenceSchema.parse(
            JSON.parse(block.reference.slice(referencePrefix.length)),
          )
        : block,
    ),
  }
}
