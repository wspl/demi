import { delay, errorMessage } from '@demicodes/utils'
import {
  BashEnvironment,
  CommandRegistry,
  shellQuote,
  type Command,
  type CommandGroup,
  type Host,
  type HostStore,
} from '@demicodes/shell'
import { z } from 'zod'
import { VIRTUAL_WORKSPACE_CWD } from '../conversation/scoped-transport'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, PrevTarget } from '../storage/control'

/** Generous buffers for the virtual-prev portable path: exports are whole-tree tars. */
const PREV_SHELL_OUTPUT_BYTES = 256 * 1024 * 1024

export interface HostCommandDeps {
  control: ControlService
  registry: RunnerRegistry
  virtualHostFor: (conversationId: string) => Promise<Host>
  hostStoreFor: (conversationId: string) => HostStore
}

/**
 * The backend-contributed `demi host` subcommand group (demi-next.md § The
 * `demi host` command). Surface: `current`, `prev shell`, `prev release`;
 * `switch` arrives with managed-host provisioning.
 */
export function createHostCommandGroup(deps: HostCommandDeps, conversationId: string): CommandGroup {
  return {
    name: 'host',
    summary: 'Execution-target operations: show the current host, reach the previous host during migration.',
    subcommands: [currentCommand(deps, conversationId), prevGroup(deps, conversationId)],
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
        lines.push(`host: virtual (files under ${VIRTUAL_WORKSPACE_CWD})`)
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
      'Run a command on the previous host: `demi host prev shell -- <argv...>`. Stdout/stderr and the exit code pass through byte-faithfully, so archives pipe cleanly (e.g. `demi host prev shell -- tar cz -C <dir> . | tar xz`).',
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
        return prev.target.kind === 'virtual'
          ? await runOnVirtualPrev(deps, conversationId, argv, ctx.io)
          : await runOnWorkspacePrev(deps, conversationId, prev.target, argv, ctx)
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

/** Real device: spawn directly and stream — real binaries, real exit codes, no buffering. */
async function runOnWorkspacePrev(
  deps: HostCommandDeps,
  conversationId: string,
  prev: Extract<PrevTarget, { kind: 'workspace' }>,
  argv: string[],
  ctx: { io: { stdout(data: string | Uint8Array): Promise<void> | void; stderr(data: string | Uint8Array): Promise<void> | void }; stdin: { bytes: Uint8Array } },
): Promise<{ exitCode: number }> {
  const host = deps.registry.hostFor({ deviceId: prev.deviceId, path: prev.path }, conversationId, deps.hostStoreFor(conversationId))
  const handle = await host.process.spawn({ command: argv[0]!, args: argv.slice(1), cwd: prev.path })
  const stdinDone = (async () => {
    if (ctx.stdin.bytes.length > 0) await handle.writeStdin(ctx.stdin.bytes)
    await handle.closeStdin()
  })().catch(() => {})
  const pumps = Promise.all([
    (async () => {
      for await (const chunk of handle.stdout) await ctx.io.stdout(chunk)
    })(),
    (async () => {
      for await (const chunk of handle.stderr) await ctx.io.stderr(chunk)
    })(),
  ])
  const exit = await handle.wait()
  await pumps.catch(() => {})
  await stdinDone
  if (exit.spawnError) {
    await ctx.io.stderr(`prev shell: ${argv[0]}: ${exit.spawnError.kind}${exit.spawnError.detail ? ` — ${exit.spawnError.detail}` : ''}\n`)
    return { exitCode: 127 }
  }
  return { exitCode: exit.exitCode ?? 1 }
}

/**
 * Virtual prev: run through a prev-side shell, where the portable command set
 * (just-bash — full `tar` included) operates over the virtual `Host.fs`.
 * Output is buffered (virtual trees are conversation-sized); the byte-faithful
 * channel is the exec result's `binaryStdout` / UTF-8 delta contract.
 */
async function runOnVirtualPrev(
  deps: HostCommandDeps,
  conversationId: string,
  argv: string[],
  io: { stdout(data: string | Uint8Array): Promise<void> | void; stderr(data: string | Uint8Array): Promise<void> | void },
): Promise<{ exitCode: number }> {
  const host = await deps.virtualHostFor(conversationId)
  const environment = new BashEnvironment({
    host,
    commands: new CommandRegistry(),
    maxOutputBytes: PREV_SHELL_OUTPUT_BYTES,
    maxBinaryBytes: PREV_SHELL_OUTPUT_BYTES,
    maxCaptureBytes: PREV_SHELL_OUTPUT_BYTES,
  })
  try {
    const script = argv.map(shellQuote).join(' ')
    let view = await environment.exec({
      script,
      ephemeral: true,
      cwd: VIRTUAL_WORKSPACE_CWD,
      timeoutMs: 60_000,
      maxOutputBytes: PREV_SHELL_OUTPUT_BYTES,
    })
    while (view.status === 'running') {
      await delay(100)
      view = await environment.status({
        commandId: view.commandId,
        stdoutOffset: 0,
        stderrOffset: 0,
        maxOutputBytes: PREV_SHELL_OUTPUT_BYTES,
      })
    }
    if (view.status !== 'exited') {
      await io.stderr('prev shell: command aborted\n')
      return { exitCode: 130 }
    }
    if (view.binaryStdout) {
      if (view.binaryStdout.truncated) {
        await io.stderr(`prev shell: output exceeded the ${PREV_SHELL_OUTPUT_BYTES}-byte buffer\n`)
        return { exitCode: 1 }
      }
      await io.stdout(view.binaryStdout.data)
    } else if (view.stdout.delta.length > 0) {
      await io.stdout(view.stdout.delta)
    }
    if (view.stderr.delta.length > 0) await io.stderr(view.stderr.delta)
    return { exitCode: view.exitCode ?? 1 }
  } finally {
    await environment.disposeAllShells().catch(() => {})
  }
}

function describePrev(prev: PrevTarget): string {
  if (prev.kind === 'virtual') return `virtual (files under ${VIRTUAL_WORKSPACE_CWD})`
  return `${prev.path} on device ${prev.deviceId}`
}
