import { z } from 'zod'
import { artifactDigestSchema, nativeTargetSchema, nativeTargetsSchema, NATIVE_PROTOCOL_VERSION } from '@demicodes/command-protocol'

/** The runner wire's version, which a runner release names. */
export const RUNNER_PROTOCOL_VERSION = 19

export const releaseDigest = artifactDigestSchema
export const releaseTarget = nativeTargetSchema
export const runnerReleaseSchema = z.object({
  release: releaseDigest,
  wire: z.literal(RUNNER_PROTOCOL_VERSION),
  commandProtocol: z.literal(NATIVE_PROTOCOL_VERSION),
  targets: nativeTargetsSchema,
}).strict()
export type RunnerRelease = z.infer<typeof runnerReleaseSchema>
