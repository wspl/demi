import { homedir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { createBackend, type InstanceMode } from './backend'
import { FirecrackerProvisioner, firecrackerConfigFromEnv } from './managed/firecracker'

async function main(): Promise<void> {
  const dataDir = process.env.DEMI_BACKEND_DATA ?? join(homedir(), '.demi', 'backend')
  const port = Number(process.env.DEMI_BACKEND_PORT ?? 3271)
  const mode = process.env.DEMI_INSTANCE_MODE
  if (mode !== 'shared' && mode !== 'isolated') throw new Error('DEMI_INSTANCE_MODE must be "shared" or "isolated" (product.md § Instance mode)')
  // Managed hosts (`managed-hosts.md`) when `DEMI_MANAGED_FIRECRACKER` names the binary; guests dial `DEMI_BACKEND_PUBLIC_URL`.
  const firecracker = firecrackerConfigFromEnv(process.env, dataDir)
  const publicUrl = process.env.DEMI_BACKEND_PUBLIC_URL
  if (firecracker && !publicUrl) throw new Error('DEMI_BACKEND_PUBLIC_URL is required with managed hosts: the URL guests dial')
  const backend = await createBackend({
    dataDir,
    port,
    mode: mode as InstanceMode,
    ...(publicUrl ? { publicUrl } : {}),
    ...(firecracker ? { managedHosts: { provisioner: new FirecrackerProvisioner(firecracker) } } : {}),
  })
  console.log(`demi-backend listening on ${backend.url} (data: ${dataDir}, ${mode} mode)`)
  if (firecracker) console.log(`managed hosts: firecracker ${firecracker.launch.mode} mode, ${firecracker.slots} slots on ${firecracker.subnet}`)
  console.log('Providers come from connections: add one via POST /api/connections (or the web UI).')

  const shutdown = () => {
    void backend.close().then(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

void main()
