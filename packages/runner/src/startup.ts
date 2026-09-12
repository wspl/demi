import { z } from 'zod'

export const runnerBackendUrlSchema = z.url({ protocol: /^(https?|wss?)$/ })
const runnerEnvironmentSchema = z.object({
  DEMI_HOME: z.string().regex(/\S/, 'must not be blank').optional(),
  DEMI_RUNNER_NAME: z.string().regex(/\S/, 'must not be blank').optional(),
  DEMI_RUNNER_MANAGED: z.enum(['0', '1']).optional(),
  DEMI_RUNNER_RECONNECT_MS: z
    .string()
    .regex(/^[0-9]+$/)
    .transform(Number)
    .pipe(z.int().min(1).max(2147483647))
    .optional(),
})

export function runnerStartupFromEnv(env: Record<string, string | undefined>) {
  const value = runnerEnvironmentSchema.parse(env)
  return {
    stateDir: value.DEMI_HOME,
    name: value.DEMI_RUNNER_NAME,
    managed: value.DEMI_RUNNER_MANAGED === '1',
    reconnect:
      value.DEMI_RUNNER_RECONNECT_MS === undefined
        ? undefined
        : {
            initialDelayMs: value.DEMI_RUNNER_RECONNECT_MS,
          },
  }
}
