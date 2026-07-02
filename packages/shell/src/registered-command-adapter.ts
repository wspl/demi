import { concatBytes, decodeUtf8, encodeUtf8 } from '@demicodes/utils'
import type { Command as ForkCommand, CommandContext as ForkCommandContext, ExecResult as ForkExecResult } from '@demicodes/just-bash/types'
import { isCommandGroup, runRegisteredCommand, type CommandAsset, type CommandIO, type CommandSpec } from './command'
import type { ShellSession } from './environment-state'
import type { AgentSessionCommandStorage } from './storage'

export function commandSpecToForkCommand(session: ShellSession, spec: CommandSpec, storage: AgentSessionCommandStorage): ForkCommand {
  return {
    name: spec.name,
    consumesStdin: treeConsumesStdin(spec),
    execute: async (args, ctx): Promise<ForkExecResult> => {
      const stdinText = decodeForkStdin(ctx.stdin)
      const io = createForwardingIO()
      const argv = [spec.name, ...args]
      try {
        const result = await runRegisteredCommand(spec, {
          argv,
          stdin: { text: stdinText },
          env: mapToRecord(ctx.env),
          cwd: ctx.cwd,
          io,
          storage,
        })
        session.accumulator.audit.push({
          kind: 'registered-command',
          name: spec.name,
          args,
          exitCode: result.exitCode,
        })
        if (result.metadata !== undefined) {
          session.accumulator.commandMetadata.push({
            kind: 'registered-command',
            name: spec.name,
            args,
            metadata: result.metadata,
          })
        }
        return { stdout: io.stdoutText(), stderr: io.stderrText(), exitCode: result.exitCode }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        session.accumulator.audit.push({
          kind: 'registered-command',
          name: spec.name,
          args,
          exitCode: 1,
        })
        return { stdout: io.stdoutText(), stderr: `${io.stderrText()}${spec.name}: ${message}\n`, exitCode: 1 }
      } finally {
        if (io.assets().length > 0) session.accumulator.assets.push(...io.assets())
      }
    },
  }
}

class ForwardingIO implements CommandIO {
  private readonly stdoutChunks: Uint8Array[] = []
  private readonly stderrChunks: Uint8Array[] = []
  private readonly assetItems: CommandAsset[] = []

  async stdout(data: string | Uint8Array): Promise<void> {
    this.stdoutChunks.push(typeof data === 'string' ? encodeUtf8(data) : data)
  }

  async stderr(data: string | Uint8Array): Promise<void> {
    this.stderrChunks.push(typeof data === 'string' ? encodeUtf8(data) : data)
  }

  asset(asset: CommandAsset): void {
    this.assetItems.push(asset)
  }

  stdoutText(): string {
    return decodeUtf8(concatBytes(this.stdoutChunks))
  }

  stderrText(): string {
    return decodeUtf8(concatBytes(this.stderrChunks))
  }

  assets(): CommandAsset[] {
    return this.assetItems
  }
}

function createForwardingIO(): ForwardingIO {
  return new ForwardingIO()
}

function treeConsumesStdin(spec: CommandSpec): boolean {
  return spec.subcommands.some((node) => (isCommandGroup(node) ? treeConsumesStdin(node) : Boolean(node.stdinField)))
}

function mapToRecord(map: Map<string, string>): Record<string, string> {
  const record: Record<string, string> = Object.create(null)
  for (const [key, value] of map) record[key] = value
  return record
}

function decodeForkStdin(stdin: ForkCommandContext['stdin']): string {
  if (!stdin) return ''
  if (stdin instanceof Uint8Array) return decodeUtf8(stdin)
  const latin1 = stdin as unknown as string
  if (!latin1) return ''
  let hasHighByte = false
  for (let i = 0; i < latin1.length; i += 1) {
    const code = latin1.charCodeAt(i)
    if (code > 0xff) return latin1
    if (code > 0x7f) hasHighByte = true
  }
  if (!hasHighByte) return latin1
  const bytes = new Uint8Array(latin1.length)
  for (let i = 0; i < latin1.length; i += 1) bytes[i] = latin1.charCodeAt(i)
  return decodeUtf8(bytes)
}
