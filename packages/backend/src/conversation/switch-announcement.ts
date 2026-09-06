import type { AgentHarness } from '@demicodes/agent'
import type { CodingState } from '@demicodes/coding-agent'
import type { RunnerRegistry } from '../runner/registry'
import type { AttachedHostRecord, ControlService, ExecutionTarget } from '../storage/control'
import { HOSTLESS_HOME } from './scoped-transport'

/**
 * Injects the context block the model needs at a turn boundary
 * (`sessions-and-targets.md`): after a switch, the departed and current
 * targets and directories, that no files were moved, and the departed host
 * under the name it stays attached as; after a change to the attached
 * hosts, the set as it now stands. Either block lists every attached host
 * with its directory. Each node observes the context revision in its own persisted transcript;
 * no shared announcement is consumed.
 */
export function switchAnnouncementPreamble(control: ControlService, registry: RunnerRegistry): AgentHarness<CodingState>['context'] {
  return async (ctx) => {
    const conversation = await control.getConversation(ctx.rootSessionId)
    if (!conversation) return null
    if (!conversation.contextVersion) return null
    const marker = `[Execution context ${conversation.contextVersion}]`
    if (ctx.transcript.blocks.some(block => block.type === 'user' && block.preamble?.includes(marker))) return null
    const attached = await control.listAttachedHosts(conversation.id)
    const lines: string[] = [marker]

    const switchDescription = conversation.lastSwitch
      ? `Previous target: ${await describe(control, conversation.lastSwitch.from)}. Current target: ${await describe(control, conversation.lastSwitch.to)}. New shells start in ${targetDirectory(conversation.lastSwitch.to, registry)}.`
      : null
    const previousSwitch = [...ctx.transcript.blocks].reverse().find(block => block.type === 'user' && block.preamble?.includes('[Execution target switched]'))
    const switchObserved = switchDescription !== null && previousSwitch?.type === 'user' && previousSwitch.preamble?.includes(switchDescription)
    if (conversation.lastSwitch && !switchObserved) {
      const { from, to } = conversation.lastSwitch
      lines.push(
        '[Execution target switched]',
        switchDescription!,
        'No files were moved: everything created earlier lives on the previous target, and file paths from before the switch — including the full outputs of earlier commands — are stale here.',
      )
      const departed = from.kind === 'hostless' ? null : attached.find((host) => host.deviceId === from.deviceId)
      if (departed) {
        const fromDir = targetDirectory(from, registry)
        lines.push(
          `The previous host stays attached as "${departed.name}": \`demi host shell --host ${departed.name} <script>\` runs a shell string there with byte-faithful stdio, starting in ${fromDir} (e.g. \`demi host shell --host ${departed.name} "tar c -C ${fromDir} ." | tar x\` pulls its files into the current directory).`,
        )
      }
      if (from.kind === 'workspace' && to.kind === 'workspace' && from.deviceId === to.deviceId) {
        lines.push(`The previous directory ${from.path} is on the same device, so it is also directly accessible from this shell.`)
      }
    } else {
      lines.push('[Attached hosts changed]')
    }
    lines.push(attachedHostsLine(attached, registry))

    return lines.join('\n')
  }
}

/** The attached hosts as one line: name, directory, online, and how to reach them. */
function attachedHostsLine(attached: AttachedHostRecord[], registry: RunnerRegistry): string {
  if (attached.length === 0) return 'Attached hosts: none. `demi host list` shows every host this conversation can reach.'
  const entries = attached.map((host) => `"${host.name}" (${registry.deviceOnline(host.deviceId) ? 'online' : 'offline'}, shells start in ${attachedDirectory(host, registry)})`)
  return `Attached hosts: ${entries.join(', ')}. \`demi host shell --host <name> <script>\` runs a shell string on one; \`demi host list\` shows every host this conversation can reach.`
}

export function attachedDirectory(host: AttachedHostRecord, registry: RunnerRegistry): string {
  return host.cwd ?? registry.deviceIdentity(host.deviceId)?.homeDir ?? 'its home directory'
}

function targetDirectory(target: ExecutionTarget, registry: RunnerRegistry): string {
  if (target.kind === 'hostless') return HOSTLESS_HOME
  if (target.kind === 'workspace') return target.path
  return registry.deviceIdentity(target.deviceId)?.homeDir ?? 'its home directory'
}

async function describe(control: ControlService, target: ExecutionTarget): Promise<string> {
  if (target.kind === 'hostless') return `the virtual environment (files under ${HOSTLESS_HOME})`
  const device = await control.getDevice(target.deviceId)
  const name = device ? `"${device.name}"` : target.deviceId
  if (target.kind === 'workspace') {
    const workspace = await control.getWorkspace(target.workspaceId)
    return `workspace "${workspace?.name ?? target.workspaceId}" — directory ${target.path} on device ${name}`
  }
  return `the machine ${name} (host ${target.deviceId})`
}
