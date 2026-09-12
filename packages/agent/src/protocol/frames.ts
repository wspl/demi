import type { z } from 'zod'
import type { Block } from '@demicodes/core'
import type { ShellCommandStatus } from '@demicodes/shell'
import type { clientFrameSchema } from './schemas'
import type { createServerFrameSchema, createTranscriptPatchSchema } from './server-schemas'

/** Both serialized directions derive their types from the ingress schemas. */
export type ClientFrame = z.infer<typeof clientFrameSchema>
export type ServerFrame<B extends Block<unknown, unknown> = Block> = z.infer<
  ReturnType<typeof createServerFrameSchema<z.ZodType<B>>>
>
export type TranscriptPatch<B extends Block<unknown, unknown> = Block> = z.infer<
  ReturnType<typeof createTranscriptPatchSchema<z.ZodType<B>>>
>
export type SubagentJob = Extract<ServerFrame, { type: 'subagent' }>['job']
export type ShellCommandStatusLike = ShellCommandStatus

/** Client events omit replication bookkeeping and include the materialized root transcript. */
export type ClientSessionEvent<B extends Block<unknown, unknown> = Block> =
  | Exclude<
      ServerFrame<B>,
      {
        type:
          | 'transcript_reset'
          | 'transcript_patch'
          | 'subagent_transcript_reset'
          | 'subagent_transcript_patch'
      }
    >
  | { type: 'transcript_reset'; blocks: B[] }
  | { type: 'transcript_patch'; patches: TranscriptPatch<B>[]; blocks: B[] }
  | { type: 'subagent_transcript_reset'; subagentId: string; blocks: B[] }
  | { type: 'subagent_transcript_patch'; subagentId: string; patches: TranscriptPatch<B>[] }
  | { type: 'disconnected' }
