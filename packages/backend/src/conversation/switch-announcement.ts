import type { AgentHarness } from '@demicodes/agent'
import type { CodingState } from '@demicodes/coding-agent'
import type { ControlService, PrevTarget } from '../storage/control'
import { VIRTUAL_WORKSPACE_CWD } from './scoped-transport'

/**
 * Injects the target-switch context block into the first round after a
 * switch: names the departed and current targets, states that no files were
 * moved, and points at the `demi host prev` pipe. One announcement per
 * switch — the prev slot's `announced` flag flips on injection.
 */
export function switchAnnouncementPreamble(control: ControlService): AgentHarness<CodingState>['preamble'] {
  return async (ctx) => {
    const conversation = await control.getConversation(ctx.agentSessionId)
    if (!conversation?.prevTarget || conversation.prevTarget.announced) return null

    const workspace = conversation.workspaceId ? await control.getWorkspace(conversation.workspaceId) : null
    const prev = conversation.prevTarget.target
    const prevDir = prev.kind === 'virtual' ? VIRTUAL_WORKSPACE_CWD : prev.path
    const currentDesc = workspace
      ? `workspace "${workspace.name}" — directory ${workspace.path} on device ${await deviceName(control, workspace.deviceId)}`
      : `the virtual environment (files under ${VIRTUAL_WORKSPACE_CWD})`
    const currentDir = workspace ? workspace.path : VIRTUAL_WORKSPACE_CWD

    const lines = [
      '[Execution target switched]',
      `Previous target: ${await describePrev(control, prev)}. Current target: ${currentDesc}. New shells start in ${currentDir}.`,
      'No files were moved: everything created earlier — including full command outputs under the artifacts directory — lives on the previous target, and file paths from before the switch are stale here.',
      `The previous target stays reachable until released: \`demi host prev shell -- <argv>\` runs a command there with byte-faithful stdio (e.g. \`demi host prev shell -- tar cz -C ${prevDir} . | tar xz\` pulls its files into the current directory), and \`demi host prev release\` gives it back once migration is done.`,
    ]
    if (prev.kind === 'workspace' && workspace && prev.deviceId === workspace.deviceId) {
      lines.push(`The previous directory ${prev.path} is on the same device, so it is also directly accessible from this shell.`)
    }

    await control.markConversationPrevAnnounced(ctx.agentSessionId)
    return lines.join('\n')
  }
}

async function describePrev(control: ControlService, prev: PrevTarget): Promise<string> {
  if (prev.kind === 'virtual') return `the virtual environment (files under ${VIRTUAL_WORKSPACE_CWD})`
  return `directory ${prev.path} on device ${await deviceName(control, prev.deviceId)}`
}

async function deviceName(control: ControlService, deviceId: string): Promise<string> {
  const device = await control.getDevice(deviceId)
  return device ? `"${device.name}"` : deviceId
}
