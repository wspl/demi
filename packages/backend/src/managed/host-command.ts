import { errorMessage } from '@demicodes/utils'
import { type Command, type CommandGroup, type CommandIO, type Host, type HostStore } from '@demicodes/shell'
import { z } from 'zod'
import { resolveExecutionTarget, targetDeviceId } from '../conversation/execution-target'
import { HOSTLESS_HOME } from '../conversation/scoped-transport'
import type { RpcTransferDestination, RunnerRegistry } from '../runner/registry'
import type { TransferBroker } from '../runner/transfers'
import type { ControlService, ExecutionTarget } from '../storage/control'

export interface HostCommandDeps {
  control: ControlService
  registry: RunnerRegistry
  transfers: TransferBroker
  virtualHostFor: (conversationId: string) => Promise<Host>
  hostStoreFor: (conversationId: string) => HostStore
}

/**
 * The backend-contributed `demi host` subcommand group (`commands.md` § The
 * `demi host` group): `list`, `current`, `shell --id`. A cross-host command
 * runs as a job on that host; its stdout comes back as a brokered transfer,
 * never over the runner sockets.
 */
export function createHostCommandGroup(deps: HostCommandDeps, conversationId: string): CommandGroup {
  return {
    name: 'host',
    summary: 'Execution-target operations: list reachable hosts, show the current one, run a command on another host.',
    subcommands: [listCommand(deps, conversationId), currentCommand(deps, conversationId), shellCommand(deps, conversationId)],
  }
}

/** A host this conversation may dispatch to: its device and the directory commands start in. */
interface ReachableHost {
  deviceId: string
  path: string
  role: 'current' | 'granted'
}

/**
 * The hosts `shell --id` accepts (`sessions-and-targets.md` § Host grants):
 * the current target's device and the conversation's grant set. A granted
 * host starts a shell in its home. The one place the check lives.
 */
async function reachableHosts(deps: HostCommandDeps, conversationId: string): Promise<ReachableHost[]> {
  const conversation = await deps.control.getConversation(conversationId)
  if (!conversation) return []
  const target = await resolveExecutionTarget(deps.control, conversation)
  const hosts: ReachableHost[] = []
  const currentDeviceId = targetDeviceId(target)
  if (currentDeviceId !== null) hosts.push({ deviceId: currentDeviceId, path: targetDirectory(deps, target), role: 'current' })
  for (const grant of await deps.control.listHostGrants(conversationId)) {
    if (grant.deviceId === currentDeviceId) continue
    hosts.push({ deviceId: grant.deviceId, path: deps.registry.deviceIdentity(grant.deviceId)?.homeDir ?? '', role: 'granted' })
  }
  return hosts
}

function targetDirectory(deps: HostCommandDeps, target: ExecutionTarget): string {
  if (target.kind === 'hostless') return HOSTLESS_HOME
  if (target.kind === 'workspace') return target.path
  return deps.registry.deviceIdentity(target.deviceId)?.homeDir ?? ''
}

function listCommand(deps: HostCommandDeps, conversationId: string): Command {
  return {
    name: 'list',
    summary: 'Hosts this conversation can reach with `demi host shell --id`: id, name, online, directory; the current one marked.',
    kind: 'rpc',
    run: async ({ io }) => {
      const hosts = await reachableHosts(deps, conversationId)
      if (hosts.length === 0) {
        await io.stdout('no hosts: this conversation runs hostless and has been granted no host\n')
        return { exitCode: 0 }
      }
      const lines = await Promise.all(
        hosts.map(async (host) => {
          const device = await deps.control.getDevice(host.deviceId)
          const online = deps.registry.deviceOnline(host.deviceId) ? 'online' : 'offline'
          return `${host.deviceId}  ${device?.name ?? '?'}  ${online}  ${host.path || '?'}  (${host.role})`
        }),
      )
      await io.stdout(`${lines.join('\n')}\n`)
      return { exitCode: 0 }
    },
  }
}

function shellCommand(deps: HostCommandDeps, conversationId: string): Command {
  return {
    name: 'shell',
    summary:
      'Run a shell string in another host\'s bash: `demi host shell --id <hostId> <script>`. The script runs in that host\'s default directory; its stdout arrives once it exits, byte-faithfully, so archives pipe cleanly (`demi host shell --id A "tar c -C /work ." | tar x`). stderr and the exit code pass through.',
    failureOutput: 'writes the reason to stderr and exits non-zero (127 when the host cannot run bash)',
    input: { id: z.string(), script: z.string() },
    positionals: ['script'],
    kind: 'rpc',
    run: async (ctx) => {
      const id = ctx.parsed.values.id as string
      const script = ctx.parsed.values.script as string
      if (script.trim() === '') {
        await ctx.io.stderr('usage: demi host shell --id <hostId> <script>\n')
        return { exitCode: 2 }
      }
      const host = (await reachableHosts(deps, conversationId)).find((candidate) => candidate.deviceId === id)
      if (!host) {
        await ctx.io.stderr(`host shell: host ${id} is not reachable from this conversation (see \`demi host list\`)\n`)
        return { exitCode: 1 }
      }
      try {
        return await runOnHost(deps, conversationId, host, script, ctx)
      } catch (error) {
        await ctx.io.stderr(`host shell: ${errorMessage(error)}\n`)
        return { exitCode: 1 }
      }
    },
  }
}

function currentCommand(deps: HostCommandDeps, conversationId: string): Command {
  return {
    name: 'current',
    summary: 'The current execution target.',
    kind: 'rpc',
    run: async ({ io }) => {
      const conversation = await deps.control.getConversation(conversationId)
      if (!conversation) {
        await io.stderr('host: this session has no conversation record\n')
        return { exitCode: 1 }
      }
      const target = await resolveExecutionTarget(deps.control, conversation)
      if (target.kind === 'hostless') {
        await io.stdout(`host: virtual (files under ${HOSTLESS_HOME})\n`)
        return { exitCode: 0 }
      }
      const device = await deps.control.getDevice(target.deviceId)
      const name = device?.name ?? target.deviceId
      const online = deps.registry.deviceOnline(target.deviceId) ? 'online' : 'offline'
      if (target.kind === 'workspace') {
        const workspace = await deps.control.getWorkspace(target.workspaceId)
        await io.stdout(`host: workspace "${workspace?.name ?? target.workspaceId}" — ${target.path} on device "${name}" (${online})\n`)
      } else {
        await io.stdout(`host: machine "${name}" (${target.deviceId}, ${online}) — ${targetDirectory(deps, target) || 'home'}\n`)
      }
      return { exitCode: 0 }
    },
  }
}

/**
 * A script on another host, as one job there (`runner.md` § Transfers): the
 * pipe's bytes are its stdin, its stderr view streams back as it runs, and
 * at exit its full stdout file is transferred — to the calling device when
 * the call was relayed from one, otherwise into this process.
 */
async function runOnHost(
  deps: HostCommandDeps,
  conversationId: string,
  target: ReachableHost,
  script: string,
  ctx: { io: CommandIO; stdin: { bytes: Uint8Array }; env: Record<string, string> },
): Promise<{ exitCode: number }> {
  if (!deps.registry.deviceOnline(target.deviceId)) {
    await ctx.io.stderr(`host shell: host ${target.deviceId} is offline\n`)
    return { exitCode: 1 }
  }
  const host = deps.registry.hostFor(target, conversationId, deps.hostStoreFor(conversationId))
  const env: Record<string, string> = { DEMI_SESSION_ID: conversationId }
  if (ctx.env.DEMI_SHELL_ID) env.DEMI_SHELL_ID = ctx.env.DEMI_SHELL_ID
  const job = host.startJob({ script, cwd: target.path, env })
  const stdinDone = (async () => {
    if (ctx.stdin.bytes.length > 0) await job.writeStdin(ctx.stdin.bytes)
    await job.closeStdin()
  })().catch(() => {})
  const view = (async () => {
    for await (const chunk of job.output) {
      if (chunk.stream === 'stderr') await ctx.io.stderr(chunk.chunk)
    }
  })()
  const exit = await job.wait()
  await view.catch(() => {})
  await stdinDone
  if (exit.spawnError) {
    await ctx.io.stderr(`host shell: ${exit.spawnError.kind}${exit.spawnError.detail ? ` — ${exit.spawnError.detail}` : ''}\n`)
    return { exitCode: 127 }
  }
  const output = exit.output
  if (output && output.stdoutBytes > 0) {
    const destination = transferDestinationOf(ctx.io)
    const transfer = deps.transfers.open(
      target.deviceId,
      destination ? { deviceId: destination.deviceId } : { consume: async (chunk) => void (await ctx.io.stdout(chunk)) },
    )
    destination?.receive(transfer.url)
    await Promise.all([deps.registry.transferSend(target.deviceId, transfer.id, output.stdoutPath, transfer.url), transfer.done])
  }
  return { exitCode: exit.exitCode ?? 1 }
}

/** The relayed-rpc io carries where its device can `GET` a transfer; any other io takes the bytes here. */
function transferDestinationOf(io: CommandIO): RpcTransferDestination | null {
  return (io as { transferDestination?: RpcTransferDestination }).transferDestination ?? null
}
