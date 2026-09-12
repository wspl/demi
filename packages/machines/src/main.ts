// The machine manager as a process: `demi-machines`. Linux runs it beside
// the backend; macOS runs it inside the Lima instance (`lima/`), where KVM
// exists. Either way the backend reaches it through `DEMI_MACHINES_SOCKET`
// and never through TCP.
import { homedir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { FirecrackerProvisioner } from './firecracker/provisioner'
import { firecrackerConfigFromEnv, MANAGED_ENV } from './firecracker/config'
import { serveMachines } from './server'

export const MACHINES_ENV = {
  socket: 'DEMI_MACHINES_SOCKET',
  data: 'DEMI_MACHINES_DATA',
} as const

async function main(): Promise<void> {
  const socketPath = process.env[MACHINES_ENV.socket]
  if (!socketPath) {
    throw new Error(`${MACHINES_ENV.socket} is required: the Unix socket the backend dials`)
  }
  const dataDir = process.env[MACHINES_ENV.data] ??
    join(homedir(), '.demi', 'machines')
  const config = firecrackerConfigFromEnv(process.env, dataDir)
  if (!config) {
    throw new Error(`${MANAGED_ENV.firecracker} is required: the Firecracker binary`)
  }
  const provisioner = new FirecrackerProvisioner(config)
  // A manager that restarted may find VMs from its last life; they are
  // stopped and their disks saved before any backend is served.
  await provisioner.reconcile()
  const server = await serveMachines({ socketPath, provisioner })
  console.log(
    `demi-machines listening on ${server.socketPath} (data: ${dataDir}, firecracker ${config.launch.mode} mode, ${config.slots} slots on ${config.subnet})`
  )

  let stopping: Promise<void> | null = null
  const shutdown = () => {
    stopping ??= server.close()
      .then(() => provisioner.close())
      .then(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

void main()
