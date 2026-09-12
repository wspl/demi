import { homedir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { createBackend } from './backend'
import { backendStartupFromEnv } from './startup'
import { RemoteProvisioner } from '@demicodes/machines'

async function main(): Promise<void> {
  const { dataDir, port, mode, webDirectory, machinesSocket, publicUrl } =
    backendStartupFromEnv(process.env, join(homedir(), '.demi', 'backend'))
  const backend = await createBackend({
    dataDir,
    webDirectory,
    port,
    mode,
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
