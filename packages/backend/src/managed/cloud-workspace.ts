import { mkdir } from 'node:fs/promises'
import { join } from 'node:path'
import { createId } from '@demicodes/utils'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, WorkspaceRecord } from '../storage/control'
import type { ManagedHosts } from './lifecycle'

export interface CloudWorkspaceDeps {
  control: ControlService
  managedHosts: ManagedHosts
  registry: RunnerRegistry
  /** Where a new host's home starts: `<stagingDir>/<workspaceId>`, empty, handed to the provisioner which owns it from then on. */
  stagingDir: string
}

/**
 * The Cloud device choice on workspace creation (`managed-hosts.md` §
 * Cloud workspace): one host owned by the workspace over an empty home,
 * and the workspace at that home. The id is chosen first so the device row
 * can name its owner before the workspace row exists; a host that never
 * comes up leaves neither — `provisionFresh` takes the device row with the
 * guest, since nothing references it.
 */
export async function createCloudWorkspace(deps: CloudWorkspaceDeps, userId: string, name: string): Promise<WorkspaceRecord> {
  const id = createId()
  const owner = { kind: 'workspace' as const, id }
  const home = join(deps.stagingDir, id)
  await mkdir(home, { recursive: true })
  const deviceId = (await deps.managedHosts.provisionFresh(owner, userId, home)).id
  const path = deps.registry.deviceIdentity(deviceId)?.homeDir
  if (!path) throw new Error(`machine ${deviceId} came online without reporting its home`)
  return deps.control.createWorkspace({ id, userId, deviceId, path, name })
}
