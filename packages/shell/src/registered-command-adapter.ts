import { concatBytes, decodeLatin1, decodeUtf8, encodeLatin1, encodeUtf8 } from '@demicodes/utils'
import type { Command as ForkCommand, CommandContext as ForkCommandContext, ExecResult as ForkExecResult } from '@demicodes/just-bash/types'
import { runRegisteredCommand, type Command, type CommandIO, type CommandStdin } from './command'
import type { ShellSession } from './environment-state'
import type { AgentSessionCommandStorage } from './storage'
import type { Host } from './host'

export function commandToForkCommand(
  session: ShellSession,
  command: Command,
  storage: AgentSessionCommandStorage,
  host: Host,
): ForkCommand {
  return {
    name: command.name,
    consumesStdin: treeConsumesStdin(command),
    execute: async (args, ctx): Promise<ForkExecResult> => {
      const stdin = decodeForkStdin(ctx.stdin)
      const io = createForwardingIO()
      const argv = [command.name, ...args]
      try {
        const result = await runRegisteredCommand(command, {
          argv,
          stdin,
          env: mapToRecord(ctx.env),
          cwd: ctx.cwd,
          io,
          storage,
          host,
        })
        session.accumulator.audit.push({
          kind: 'registered-command',
          name: command.name,
          args,
          exitCode: result.exitCode,
        })
        if (result.metadata !== undefined) {
          session.accumulator.commandMetadata.push({
            kind: 'registered-command',
            name: command.name,
            args,
            metadata: result.metadata,
          })
        }
        return { stdout: io.stdoutLatin1(), stdoutKind: 'bytes', stderr: io.stderrText(), exitCode: result.exitCode }
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        session.accumulator.audit.push({
          kind: 'registered-command',
          name: command.name,
          args,
          exitCode: 1,
        })
        return {
          stdout: io.stdoutLatin1(),
          stdoutKind: 'bytes',
          stderr: `${io.stderrText()}${command.name}: ${message}\n`,
          exitCode: 1,
        }
      }
    },
  }
}

/** Collects command output as raw bytes; stdout stays byte-clean for the pipe. */
class ForwardingIO implements CommandIO {
  private readonly stdoutChunks: Uint8Array[] = []
  private readonly stderrChunks: Uint8Array[] = []

  async stdout(data: string | Uint8Array): Promise<void> {
    this.stdoutChunks.push(typeof data === 'string' ? encodeUtf8(data) : data)
  }

  async stderr(data: string | Uint8Array): Promise<void> {
    this.stderrChunks.push(typeof data === 'string' ? encodeUtf8(data) : data)
  }

  stdoutLatin1(): string {
    return decodeLatin1(concatBytes(this.stdoutChunks))
  }

  stderrText(): string {
    return decodeUtf8(concatBytes(this.stderrChunks))
  }
}

function createForwardingIO(): ForwardingIO {
  return new ForwardingIO()
}

function treeConsumesStdin(command: Command): boolean {
  if (command.stdinField) return true
  return command.subcommands?.some(treeConsumesStdin) ?? false
}

function mapToRecord(map: Map<string, string>): Record<string, string> {
  const record: Record<string, string> = Object.create(null)
  for (const [key, value] of map) record[key] = value
  return record
}

/** Pipes hand stdin over as a latin1-packed byte string; expose bytes and a UTF-8 text view. */
function decodeForkStdin(stdin: ForkCommandContext['stdin']): CommandStdin {
  if (!stdin) return { text: '', bytes: new Uint8Array(0) }
  if (stdin instanceof Uint8Array) return { text: decodeUtf8(stdin), bytes: stdin }
  const latin1 = stdin as unknown as string
  if (!latin1) return { text: '', bytes: new Uint8Array(0) }
  const bytes = encodeLatin1(latin1)
  let hasHighByte = false
  let hasWideChar = false
  for (let i = 0; i < latin1.length; i += 1) {
    const code = latin1.charCodeAt(i)
    if (code > 0xff) hasWideChar = true
    else if (code > 0x7f) hasHighByte = true
  }
  // Already-Unicode text (wide chars) passes through; latin1-packed UTF-8 decodes.
  if (hasWideChar) return { text: latin1, bytes: encodeUtf8(latin1) }
  if (!hasHighByte) return { text: latin1, bytes }
  return { text: decodeUtf8(bytes), bytes }
}
