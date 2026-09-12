import { z } from 'zod'
import { firecrackerConfigFromEnv, MANAGED_ENV } from './firecracker/config'

const pathSchema = z.string().regex(/\S/, 'must not be blank')
const machinesEnvironmentSchema = z.object({
  DEMI_MACHINES_SOCKET: pathSchema,
  DEMI_MACHINES_DATA: pathSchema.optional(),
})

export function machinesStartupFromEnv(
  env: Record<string, string | undefined>,
  defaultDataDir: string,
) {
  const value = machinesEnvironmentSchema.parse(env)
  const dataDir = value.DEMI_MACHINES_DATA ?? defaultDataDir
  const config = firecrackerConfigFromEnv(env, dataDir)
  if (!config) {
    throw new Error(
      `${MANAGED_ENV.firecracker} is required: the Firecracker binary`,
    )
  }
  return { socketPath: value.DEMI_MACHINES_SOCKET, dataDir, config }
}
