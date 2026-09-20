// The machine manager as a process: `demi-machines`. Linux runs it beside
// the backend; macOS runs it inside the Lima instance (`lima/`), where KVM
// exists. Either way the backend reaches it through `DEMI_MACHINES_SOCKET`
// and never through TCP.
import { constants } from 'node:fs'
import { copyFile, mkdir, rm, writeFile } from 'node:fs/promises'
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

/**
 * Whether this filesystem clones a file instead of copying it
 * (`managed-hosts.md` § Lifecycle and capacity). Every image copy asks for a
 * clone; on a filesystem without them each machine pays for a whole image.
 */
async function clones(dataDir: string): Promise<boolean> {
  const probe = join(dataDir, '.clone-probe')
  try {
    await mkdir(dataDir, { recursive: true })
    await writeFile(probe, 'probe')
    // FICLONE_FORCE fails rather than falling back, which is the question.
    await copyFile(probe, `${probe}-clone`, constants.COPYFILE_FICLONE_FORCE)
    return true
  } catch {
    return false
  } finally {
    await rm(probe, { force: true })
    await rm(`${probe}-clone`, { force: true })
  }
}

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
    `demi-machines listening on ${server.socketPath} (data: ${dataDir}${
      await clones(dataDir) ? '' : ', which does not clone files: every image is copied in full'
    }, firecracker ${config.launch.mode} mode, ${config.slots} slots on ${config.subnet})`
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
