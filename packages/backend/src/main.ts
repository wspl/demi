import { loadChangeObjects } from './storage/change-config'
import { loadNativeArtifacts } from './runner/artifacts/config'
import { homedir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { z } from 'zod'
import { createBackend } from './backend'
import { RemoteProvisioner } from '@demicodes/machines'

/**
 * The environment the production backend starts from. Everything the process
 * needs is named here, with its default, so a misspelled or unusable value
 * fails at startup with the variable's name instead of surfacing later as a
 * NaN port or an unset mode.
 *
 * Managed hosts (`managed-hosts.md`) are on when `DEMI_MACHINES_SOCKET` names
 * the machine manager's Unix socket; guests dial `DEMI_BACKEND_PUBLIC_URL`,
 * which is therefore required with it.
 */
const backendEnvSchema = z
  .object({
    DEMI_BACKEND_DATA: z
      .string()
      .min(1)
      .default(() => join(homedir(), '.demi', 'backend')),
    DEMI_BACKEND_PORT: z.coerce.number().int().min(1).max(65535).default(3271),
    DEMI_INSTANCE_MODE: z.enum(
      ['shared', 'isolated'],
      'must be "shared" or "isolated" (product.md § Instance mode)',
    ),
    DEMI_MACHINES_SOCKET: z.string().min(1).optional(),
    DEMI_BACKEND_PUBLIC_URL: z.url().optional(),
    DEMI_NATIVE_CONFIG: z
      .string()
      .min(1, 'must name the native release and object storage configuration'),
    DEMI_WEB_DIRECTORY: z.string().min(1).optional(),
  })
  .refine(
    (env) => !env.DEMI_MACHINES_SOCKET || env.DEMI_BACKEND_PUBLIC_URL,
    {
      path: ['DEMI_BACKEND_PUBLIC_URL'],
      message: 'required with managed hosts: the URL guests dial',
    },
  )

async function main(): Promise<void> {
  const env = backendEnvSchema.parse(process.env)
  const dataDir = env.DEMI_BACKEND_DATA
  const machinesSocket = env.DEMI_MACHINES_SOCKET
  const publicUrl = env.DEMI_BACKEND_PUBLIC_URL
  const publishing = new AbortController()
  const abortPublication = () => publishing.abort()
  process.once('SIGINT', abortPublication)
  process.once('SIGTERM', abortPublication)
  const nativeCommands = await loadNativeArtifacts(
    env.DEMI_NATIVE_CONFIG,
    publishing.signal
  ).finally(() => {
    process.off('SIGINT', abortPublication)
    process.off('SIGTERM', abortPublication)
  })
  const changeStore = process.env.DEMI_CHANGE_STORE_CONFIG
    ? await loadChangeObjects(process.env.DEMI_CHANGE_STORE_CONFIG).catch(error => {
        nativeCommands.close()
        throw error
      })
    : undefined
  const backend = await createBackend({
    changeObjects: changeStore?.objects,
    nativeCommands,
    dataDir,
    webDirectory: env.DEMI_WEB_DIRECTORY,
    port: env.DEMI_BACKEND_PORT,
    mode: env.DEMI_INSTANCE_MODE,
    ...(publicUrl ? { publicUrl } : {}),
    ...(machinesSocket
      ? { managedHosts: { provisioner: new RemoteProvisioner({ socketPath: machinesSocket }) } }
      : {}),
  }).catch(error => {
    nativeCommands.close()
    changeStore?.close()
    throw error
  })
  console.log(
    `demi-backend listening on ${backend.url} (data: ${dataDir}, ${env.DEMI_INSTANCE_MODE} mode)`
  )
  if (machinesSocket)
    console.log(`managed hosts: machine manager at ${machinesSocket}`)
  console.log(
    'Providers come from providers: add one via POST /api/providers (or the web UI).'
  )

  const shutdown = () => {
    void backend.close().finally(() => {
      nativeCommands.close()
      changeStore?.close()
    }).then(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

void main()
