import { z } from 'zod'
import { artifactDigestSchema, nativeArtifactSchema, nativeTargetSchema, NATIVE_PROTOCOL_VERSION } from '@demicodes/command-protocol'
import { RUNNER_PROTOCOL_VERSION } from './messages'

export const releaseDigest = artifactDigestSchema
export const releaseTarget = nativeTargetSchema
export const runnerReleaseSchema = z.object({
  release: releaseDigest,
  wire: z.literal(RUNNER_PROTOCOL_VERSION),
  commandProtocol: z.literal(NATIVE_PROTOCOL_VERSION),
  targets: z.record(releaseTarget, nativeArtifactSchema),
}).strict()
export type RunnerRelease = z.infer<typeof runnerReleaseSchema>
