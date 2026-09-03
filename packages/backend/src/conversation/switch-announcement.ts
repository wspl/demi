import type { AgentHarness } from '@demicodes/agent'
import type { CodingState } from '@demicodes/coding-agent'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, ExecutionTarget } from '../storage/control'
import { HOSTLESS_HOME } from './scoped-transport'

/**
 * Injects the target-switch context block into the first round after a
 * switch (`sessions-and-targets.md` § Switching): names the departed and
 * current targets and directories, states that no files were moved, and
 * points at `demi host shell --id` for the departed host, which the switch
 * granted. One announcement per switch — the pending switch is cleared on
 * injection.
 */
export function switchAnnouncementPreamble(control: ControlService, registry: RunnerRegistry): AgentHarness<CodingState>['preamble'] {
  return async (ctx) => {
    const conversation = await control.getConversation(ctx.agentSessionId)
    if (!conversation?.pendingSwitch) return null
    const { from, to } = conversation.pendingSwitch

    const fromDir = targetDirectory(from, registry)
    const lines = [
      '[Execution target switched]',
      `Previous target: ${await describe(control, from)}. Current target: ${await describe(control, to)}. New shells start in ${targetDirectory(to, registry)}.`,
      'No files were moved: everything created earlier lives on the previous target, and file paths from before the switch — including the full outputs of earlier commands — are stale here.',
    ]
    if (from.kind !== 'hostless') {
      lines.push(
        `The previous host stays reachable: \`demi host shell --id ${from.deviceId} <script>\` runs a shell string there with byte-faithful stdio, starting in its home directory (e.g. \`demi host shell --id ${from.deviceId} "tar c -C ${fromDir} ." | tar x\` pulls its files into the current directory; \`demi host list\` shows every reachable host).`,
      )
    }
    if (from.kind === 'workspace' && to.kind === 'workspace' && from.deviceId === to.deviceId) {
      lines.push(`The previous directory ${from.path} is on the same device, so it is also directly accessible from this shell.`)
    }

    await control.clearPendingSwitch(ctx.agentSessionId)
    return lines.join('\n')
  }
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
