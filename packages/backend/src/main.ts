import { loadChangeObjects } from './storage/change-config'
import { loadNativeArtifacts } from './runner/artifacts/config'
import { homedir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { createBackend } from './backend'
import type { InstanceMode } from './auth/identity'
import { RemoteProvisioner } from '@demicodes/machines'

async function main(): Promise<void> {
  const dataDir = process.env.DEMI_BACKEND_DATA ??
    join(homedir(), '.demi', 'backend')
  const port = Number(process.env.DEMI_BACKEND_PORT ?? 3271)
  const mode = process.env.DEMI_INSTANCE_MODE
  if (mode !== 'shared' && mode !== 'isolated')
    throw new Error(
      'DEMI_INSTANCE_MODE must be "shared" or "isolated" (product.md § Instance mode)'
    )
  // Managed hosts (`managed-hosts.md`) when `DEMI_MACHINES_SOCKET` names the
  // machine manager's Unix socket; guests dial `DEMI_BACKEND_PUBLIC_URL`.
  const machinesSocket = process.env.DEMI_MACHINES_SOCKET
  const publicUrl = process.env.DEMI_BACKEND_PUBLIC_URL
  if (machinesSocket && !publicUrl)
    throw new Error(
      'DEMI_BACKEND_PUBLIC_URL is required with managed hosts: the URL guests dial'
    )
  const nativeConfig = process.env.DEMI_NATIVE_CONFIG
  if (!nativeConfig)
    throw new Error('DEMI_NATIVE_CONFIG must name the native release and object storage configuration')
  const publishing = new AbortController()
  const abortPublication = () => publishing.abort()
  process.once('SIGINT', abortPublication)
  process.once('SIGTERM', abortPublication)
  const nativeCommands = await loadNativeArtifacts(nativeConfig, publishing.signal).finally(() => {
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
    webDirectory: process.env.DEMI_WEB_DIRECTORY,
    port,
    mode: mode as InstanceMode,
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
    `demi-backend listening on ${backend.url} (data: ${dataDir}, ${mode} mode)`
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
