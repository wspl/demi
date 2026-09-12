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
  const backend = await createBackend({
    dataDir,
    webDirectory: process.env.DEMI_WEB_DIRECTORY,
    port,
    mode: mode as InstanceMode,
    ...(publicUrl ? { publicUrl } : {}),
    ...(machinesSocket
      ? { managedHosts: { provisioner: new RemoteProvisioner({ socketPath: machinesSocket }) } }
      : {}),
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
    void backend.close().then(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

void main()
