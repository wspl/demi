import { errorMessage, noop } from '@demicodes/utils'
import { type Command, type CommandGroup, type CommandIO, type CommandRunContext, type Host, type HostStore } from '@demicodes/shell'
import { z } from 'zod'
import { resolveExecutionTarget, targetDeviceId } from '../conversation/execution-target'
import { HOSTLESS_HOME } from '../conversation/scoped-transport'
import type { RunnerRegistry } from '../runner/registry'
import { relayedPipesOf, type Pipe, type PipeBroker } from '../runner/pipes'
import type { ControlService, ExecutionTarget } from '../storage/control'
import type { ManagedHosts } from './lifecycle'

export interface HostCommandDeps {
  control: ControlService
  registry: RunnerRegistry
  pipes: PipeBroker
  /** Wakes a hibernated managed host on `shell --id` (`sessions-and-targets.md` § Host grants); null when the backend provisions none. */
  managedHosts: ManagedHosts | null
  virtualHostFor: (conversationId: string) => Promise<Host>
  hostStoreFor: (conversationId: string) => HostStore
}

/**
 * The backend-contributed `demi host` subcommand group (`commands.md` § The
 * `demi host` group): `list`, `current`, `shell --id`. A cross-host command
 * runs as a job on that host with the caller's stdin and stdout attached to
 * the job's as pipes (`runner.md` § Pipes), never over the runner sockets.
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
      'Run a shell string in another host\'s bash: `demi host shell --id <hostId> <script>`. The script runs in that host\'s default directory with this command\'s stdin and stdout, byte-faithfully and streaming, so archives pipe cleanly both ways (`demi host shell --id A "tar c -C /work ." | tar x`, `tar c . | demi host shell --id A "tar x -C /work"`). stderr and the exit code pass through.',
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
 * A script on another host, as one job there (`runner.md` § Pipes): the
 * caller's stdin and stdout become the job's fd 0 and fd 1, both streaming
 * while it runs. A caller on a device has its pipes already minted by the
 * relay — their far ends are simply named as the job's host, so the bytes
 * go device to device through the broker; a hostless caller's ends are this
 * process's own streams. The stderr view and the exit code pass through.
 */
async function runOnHost(
  deps: HostCommandDeps,
  conversationId: string,
  target: ReachableHost,
  script: string,
  ctx: Pick<CommandRunContext, 'io' | 'stdin' | 'env'>,
): Promise<{ exitCode: number }> {
  const device = await deps.control.getDevice(target.deviceId)
  if (device?.kind === 'managed' && deps.managedHosts) await deps.managedHosts.ensureRunning(device)
  if (!deps.registry.deviceOnline(target.deviceId)) {
    await ctx.io.stderr(`host shell: host ${target.deviceId} is offline\n`)
    return { exitCode: 1 }
  }
  const host = deps.registry.hostFor(target, conversationId, deps.hostStoreFor(conversationId))
  const env: Record<string, string> = { DEMI_SESSION_ID: conversationId }
  if (ctx.env.DEMI_SHELL_ID) env.DEMI_SHELL_ID = ctx.env.DEMI_SHELL_ID
  const { stdin, stdout } = attachEnds(deps.pipes, ctx, target.deviceId)
  const job = host.startJob({ script, cwd: target.path, env, ...(stdin ? { stdin: stdin.ref() } : {}), stdout: stdout.ref() })
  const view = (async () => {
    for await (const chunk of job.output) {
      if (chunk.stream === 'stderr') await ctx.io.stderr(chunk.chunk)
    }
  })()
  const exit = await job.wait()
  await view.catch(noop)
  await stdout.done.catch((error: unknown) => ctx.io.stderr(`host shell: stdout ${errorMessage(error)}\n`))
  stdin?.done.catch(noop)
  if (exit.spawnError) {
    await ctx.io.stderr(`host shell: ${exit.spawnError.kind}${exit.spawnError.detail ? ` — ${exit.spawnError.detail}` : ''}\n`)
    return { exitCode: 127 }
  }
  return { exitCode: exit.exitCode ?? 1 }
}

/**
 * The pipes the job's ends attach to. Relayed from a device: the call's own
 * pipes, their far ends named as the job's device. In this process: a pipe
 * fed from the caller's stdin stream, and one drained into its stdout.
 */
function attachEnds(broker: PipeBroker, ctx: Pick<CommandRunContext, 'io' | 'stdin'>, deviceId: string): { stdin: Pipe | null; stdout: Pipe } {
  const relayed = relayedPipesOf(ctx.io)
  if (relayed) {
    relayed.stdin?.sinkTo(deviceId)
    relayed.stdout.sourceFrom(deviceId)
    return relayed
  }
  let stdin: Pipe | null = null
  if (ctx.stdin) {
    stdin = broker.open(undefined, { deviceId })
    const writer = stdin.writer()
    void (async () => {
      try {
        for await (const chunk of ctx.stdin!) await writer.write(chunk)
        writer.end()
      } catch (error) {
        writer.fail(error)
      }
    })()
  }
  const stdout = broker.open({ deviceId })
  void (async () => {
    try {
      for await (const chunk of stdout.stream()) await ctx.io.stdout(chunk)
    } catch {
      // Reported through `done` by the caller.
    }
  })()
  return { stdin, stdout }
}
