import { errorMessage } from '@demicodes/utils'
import { shellQuote, type Command, type CommandGroup, type CommandIO, type Host, type HostStore } from '@demicodes/shell'
import { z } from 'zod'
import { HOSTLESS_HOME } from '../conversation/scoped-transport'
import type { RpcTransferDestination, RunnerRegistry } from '../runner/registry'
import type { TransferBroker } from '../runner/transfers'
import type { ControlService, PrevTarget } from '../storage/control'

export interface HostCommandDeps {
  control: ControlService
  registry: RunnerRegistry
  transfers: TransferBroker
  virtualHostFor: (conversationId: string) => Promise<Host>
  hostStoreFor: (conversationId: string) => HostStore
}

/**
 * The backend-contributed `demi host` subcommand group (`commands.md` § The
 * `demi host` group): `list`, `current`, `shell --id`, `prev shell`, `prev
 * release`. A cross-host command runs as a job on that host; its stdout
 * comes back as a brokered transfer, never over the runner sockets.
 */
export function createHostCommandGroup(deps: HostCommandDeps, conversationId: string): CommandGroup {
  return {
    name: 'host',
    summary: 'Execution-target operations: list reachable hosts, show the current one, run a command on another host.',
    subcommands: [listCommand(deps, conversationId), currentCommand(deps, conversationId), shellCommand(deps, conversationId), prevGroup(deps, conversationId)],
  }
}

/** A host this conversation may dispatch to besides running there: its device and the directory commands start in. */
interface ReachableHost {
  deviceId: string
  path: string
  role: 'current' | 'prev'
}

/**
 * The hosts `shell --id` accepts: the current target's device and the
 * previous target's. The grant table (`sessions-and-targets.md` § Host
 * grants) widens this set; the check stays this one function.
 */
async function reachableHosts(deps: HostCommandDeps, conversationId: string): Promise<ReachableHost[]> {
  const conversation = await deps.control.getConversation(conversationId)
  if (!conversation) return []
  const hosts: ReachableHost[] = []
  const workspace = conversation.workspaceId ? await deps.control.getWorkspace(conversation.workspaceId) : null
  if (workspace) hosts.push({ deviceId: workspace.deviceId, path: workspace.path, role: 'current' })
  const prev = conversation.prevTarget?.target
  if (prev?.kind === 'workspace') hosts.push({ deviceId: prev.deviceId, path: prev.path, role: 'prev' })
  return hosts
}

function listCommand(deps: HostCommandDeps, conversationId: string): Command {
  return {
    name: 'list',
    summary: 'Hosts this conversation can reach with `demi host shell --id`: id, name, online, and the current one marked.',
    kind: 'rpc',
    run: async ({ io }) => {
      const hosts = await reachableHosts(deps, conversationId)
      if (hosts.length === 0) {
        await io.stdout('no hosts: this conversation runs hostless and has no previous host\n')
        return { exitCode: 0 }
      }
      const lines = await Promise.all(
        hosts.map(async (host) => {
          const device = await deps.control.getDevice(host.deviceId)
          const online = deps.registry.deviceOnline(host.deviceId) ? 'online' : 'offline'
          return `${host.deviceId}  ${device?.name ?? '?'}  ${online}  ${host.path}${host.role === 'current' ? '  (current)' : '  (previous)'}`
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
    summary: 'The current execution target, and the previous one while a migration is in progress.',
    kind: 'rpc',
    run: async ({ io }) => {
      const conversation = await deps.control.getConversation(conversationId)
      if (!conversation) {
        await io.stderr('host: this session has no conversation record\n')
        return { exitCode: 1 }
      }
      const lines: string[] = []
      const workspace = conversation.workspaceId ? await deps.control.getWorkspace(conversation.workspaceId) : null
      if (workspace) {
        const device = await deps.control.getDevice(workspace.deviceId)
        const online = deps.registry.deviceOnline(workspace.deviceId) ? 'online' : 'offline'
        lines.push(`host: workspace "${workspace.name}" — ${workspace.path} on device "${device?.name ?? workspace.deviceId}" (${online})`)
      } else {
        lines.push(`host: virtual (files under ${HOSTLESS_HOME})`)
      }
      if (conversation.prevTarget) {
        lines.push(`prev: ${describePrev(conversation.prevTarget.target)} — reachable via \`demi host prev shell\` until \`demi host prev release\``)
      }
      await io.stdout(`${lines.join('\n')}\n`)
      return { exitCode: 0 }
    },
  }
}

function prevGroup(deps: HostCommandDeps, conversationId: string): CommandGroup {
  return {
    name: 'prev',
    summary: 'The previous execution target, reachable during migration.',
    subcommands: [prevShellCommand(deps, conversationId), prevReleaseCommand(deps, conversationId)],
  }
}

function prevShellCommand(deps: HostCommandDeps, conversationId: string): Command {
  return {
    name: 'shell',
    summary:
      'Run a command on the previous host: `demi host prev shell -- <argv...>`. Stdout/stderr and the exit code pass through byte-faithfully, so archives pipe cleanly (e.g. `demi host prev shell -- tar cz -C <dir> . | tar xz`); on a machine the stdout arrives once the command exits.',
    failureOutput: 'writes the reason to stderr and exits non-zero (127 when the previous host cannot run the command)',
    input: { argv: z.array(z.string()).optional() },
    restField: 'argv',
    kind: 'rpc',
    run: async (ctx) => {
      const argv = ctx.parsed.values.argv as string[] | undefined
      if (!argv || argv.length === 0) {
        await ctx.io.stderr('usage: demi host prev shell -- <argv...>\n')
        return { exitCode: 2 }
      }
      const conversation = await deps.control.getConversation(conversationId)
      const prev = conversation?.prevTarget
      if (!prev) {
        await ctx.io.stderr('prev host released\n')
        return { exitCode: 1 }
      }
      try {
        if (prev.target.kind === 'virtual') {
          // The hostless files are placed on the machine by the switch itself
          // (`sessions-and-targets.md` § Upgrading); nothing runs "on" the hostless side.
          await ctx.io.stderr('prev shell: the previous target was hostless; its files were placed here by the switch\n')
          return { exitCode: 1 }
        }
        return await runOnHost(deps, conversationId, { deviceId: prev.target.deviceId, path: prev.target.path, role: 'prev' }, argv.map(shellQuote).join(' '), ctx)
      } catch (error) {
        await ctx.io.stderr(`prev shell: ${errorMessage(error)}\n`)
        return { exitCode: 1 }
      }
    },
  }
}

function prevReleaseCommand(deps: HostCommandDeps, conversationId: string): Command {
  return {
    name: 'release',
    summary: 'Give the previous host back once migration is done; `prev shell` stops working afterwards.',
    kind: 'rpc',
    run: async ({ io }) => {
      const conversation = await deps.control.getConversation(conversationId)
      if (!conversation?.prevTarget) {
        await io.stderr('prev host already released\n')
        return { exitCode: 1 }
      }
      await deps.control.clearConversationPrev(conversationId)
      await io.stdout('prev host released\n')
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

function describePrev(prev: PrevTarget): string {
  if (prev.kind === 'virtual') return `virtual (files under ${HOSTLESS_HOME})`
  return `${prev.path} on device ${prev.deviceId}`
}
