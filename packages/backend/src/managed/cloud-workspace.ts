import { createId } from '@demicodes/utils'
import { administrativeStore } from '../storage/host-store'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, WorkspaceRecord } from '../storage/control'
import type { ManagedHosts } from './lifecycle'

export interface CloudWorkspaceDeps { control: ControlService; managedHosts: ManagedHosts; registry: RunnerRegistry }

/** Projects are directories on the user's shared device. */
export async function createCloudWorkspace(deps: CloudWorkspaceDeps, userId: string, name: string): Promise<WorkspaceRecord> {
  const id = createId()
  const device = await deps.control.getOrCreateCloudDevice(userId)
  const release = await deps.managedHosts.enter(device)
  try {
    const home = deps.registry.deviceIdentity(device.id)?.homeDir
    if (!home) throw new Error('Cloud did not report its home directory')
    const path = `${home}/projects/${id}`
    const host = deps.registry.hostFor({ deviceId: device.id, path }, `workspace-${id}`, administrativeStore)
    await host.fs.mkdir(path, { recursive: true })
    return await deps.control.createWorkspace({ id, userId, deviceId: device.id, path, name })
  } finally { release() }
}
