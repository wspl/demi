import { z } from 'zod'
import { RUNNER_PROTOCOL_VERSION } from './messages'
import { LOCAL } from './local'

export const releaseDigest = z.string().regex(/^[a-f0-9]{64}$/)
export const releaseTarget = z.enum(['macos-arm64', 'macos-x64', 'linux-arm64', 'linux-x64'])
export const runnerReleaseSchema = z.object({
  release: releaseDigest,
  wire: z.literal(RUNNER_PROTOCOL_VERSION),
  local: z.literal(LOCAL.version),
  targets: z.partialRecord(releaseTarget, z.object({ runner: releaseDigest, client: releaseDigest }).strict()),
}).strict()
export type RunnerRelease = z.infer<typeof runnerReleaseSchema>
