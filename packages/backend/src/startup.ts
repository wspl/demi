import { z } from 'zod'
import { instanceModeSchema } from '@demicodes/product-contracts'

const pathSchema = z.string().regex(/\S/, 'must not be blank')
const startupEnvironmentSchema = z
  .object({
    DEMI_BACKEND_DATA: pathSchema.optional(),
    DEMI_BACKEND_PORT: z
      .string()
      .regex(/^[0-9]+$/)
      .transform(Number)
      .pipe(z.int().min(1).max(65535))
      .optional(),
    DEMI_INSTANCE_MODE: instanceModeSchema,
    DEMI_WEB_DIRECTORY: pathSchema.optional(),
    DEMI_MACHINES_SOCKET: pathSchema.optional(),
    DEMI_BACKEND_PUBLIC_URL: z.url({ protocol: /^https?$/ }).optional(),
  })
  .superRefine((value, context) => {
    if (
      value.DEMI_MACHINES_SOCKET !== undefined &&
      value.DEMI_BACKEND_PUBLIC_URL === undefined
    ) {
      context.addIssue({
        code: 'custom',
        path: ['DEMI_BACKEND_PUBLIC_URL'],
        message: 'required with DEMI_MACHINES_SOCKET: the URL guests dial',
      })
    }
  })

export function backendStartupFromEnv(
  env: Record<string, string | undefined>,
  defaultDataDir: string,
) {
  const value = startupEnvironmentSchema.parse(env)
  return {
    dataDir: value.DEMI_BACKEND_DATA ?? defaultDataDir,
    port: value.DEMI_BACKEND_PORT ?? 3271,
    mode: value.DEMI_INSTANCE_MODE,
    webDirectory: value.DEMI_WEB_DIRECTORY,
    machinesSocket: value.DEMI_MACHINES_SOCKET,
    publicUrl: value.DEMI_BACKEND_PUBLIC_URL,
  }
}
