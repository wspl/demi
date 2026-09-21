// Privileged Linux Cloud manager. The service supplies a private mount namespace.
import { mkdir, open, readFile, readlink, writeFile } from 'node:fs/promises'
import { parseArgs } from 'node:util'
import { join } from 'node:path'
import { dlopen, FFIType } from 'bun:ffi'
import { GVisorProvisioner } from './gvisor/provisioner'
import { gvisorConfigFromEnv, RUNTIME_RELEASE } from './gvisor/config'
import { missingImageTools, requireTool } from './gvisor/image-tools'
import { MountNamespace } from './gvisor/namespace'
import { recoverStorageProbes, verifyStorage } from './gvisor/preflight'
import { serveMachines } from './server'

async function main(): Promise<void> {
  const { values } = parseArgs({ options: { recover: { type: 'boolean', default: false }, 'recover-namespace': { type: 'boolean', default: false } } })
  if (process.platform !== 'linux' || process.getuid?.() !== 0) throw new Error('Cloud manager requires privileged Linux service execution')
  const socketPath = process.env.DEMI_MACHINES_SOCKET
  if (!socketPath) throw new Error('DEMI_MACHINES_SOCKET is required')
  const dataDir = process.env.DEMI_MACHINES_DATA ?? '/var/lib/demi-machines'
  const config = gvisorConfigFromEnv(process.env, dataDir)
  if (await readlink('/proc/self/ns/mnt') === await readlink('/proc/1/ns/mnt')) {
    throw new Error('Cloud manager requires a private mount namespace (systemd PrivateMounts=yes)')
  }
  const missing = missingImageTools()
  if (missing.length) throw new Error(`Cloud manager needs: ${missing.join(', ')}`)
  const version = await requireTool(config.runsc, ['--version'])
  if (!version.includes(RUNTIME_RELEASE)) throw new Error(`Cloud requires runsc ${RUNTIME_RELEASE}`)
  for (const directory of [dataDir, config.runtimeDir]) await mkdir(directory, { recursive: true, mode: 0o700 })
  if (values['recover-namespace']) {
    // The lock-owning parent invokes this only inside the previous manager's namespace.
    await recoverStorageProbes(config)
    await new GVisorProvisioner(config).reconcile()
    return
  }
  // flock stays tied to these open descriptors and is released even on process death.
  const libc = dlopen('libc.so.6', { flock: { args: [FFIType.i32, FFIType.i32], returns: FFIType.i32 } })
  const locks = []
  try {
    for (const path of [join(dataDir, 'manager.lock'), join(config.runtimeDir, 'manager.lock')]) {
      const file = await open(path, 'a', 0o600)
      locks.push(file)
      if (libc.symbols.flock(file.fd, 2 | 4) !== 0) throw new Error(`Another Cloud manager owns ${path}`)
    }
    const namespace = new MountNamespace(config)
    await namespace.recover()
    await recoverStorageProbes(config)
    const controllers = (await readFile('/sys/fs/cgroup/cgroup.controllers', 'utf8')).trim().split(/\s+/)
    for (const controller of ['cpu', 'memory', 'pids']) {
      if (!controllers.includes(controller)) throw new Error(`Missing cgroup v2 ${controller} controller`)
    }
    await writeFile('/sys/fs/cgroup/cgroup.subtree_control', '+cpu +memory +pids')
    await mkdir('/sys/fs/cgroup/demi-cloud', { recursive: true })
    await writeFile('/sys/fs/cgroup/demi-cloud/cgroup.subtree_control', '+cpu +memory +pids')
    const provisioner = new GVisorProvisioner(config)
    await provisioner.reconcile()
    if (values.recover) return
    await namespace.pin()
    await verifyStorage(config)
    const server = await serveMachines({ socketPath, provisioner, socketMode: 0o660 })
    console.log(`demi-machines: gVisor/systrap ready at ${socketPath}`)
    await new Promise<void>((resolve, reject) => {
      let stopping = false
      const shutdown = () => {
        if (stopping) return
        stopping = true
        process.off('SIGINT', shutdown)
        process.off('SIGTERM', shutdown)
        void server.close().then(() => provisioner.close()).then(resolve, reject)
      }
      process.on('SIGINT', shutdown)
      process.on('SIGTERM', shutdown)
    })
    await namespace.release()
  } finally {
    for (const file of locks) await file.close()
    libc.close()
  }
}

void main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
