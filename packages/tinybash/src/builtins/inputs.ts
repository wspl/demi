import { checkCancelled, type BuiltinContext } from './io'
import { strerror } from './errors'
import { bytesStream } from '@demicodes/utils'

export interface Input {
  /** The operand as given; `standard input` for `-` or no operands. */
  name: string
  stream: AsyncIterable<Uint8Array>
  /** Whether reading the stream failed part-way (a directory redirected to stdin); reported already. */
  readFailed: () => boolean
}

/**
 * The command's stdin with a read error — a directory redirected in — reported
 * once through `report`, after which the input simply ends: the tool goes on
 * to its next operand and sets its exit status from `failed()`.
 */
export function guardedStdin(ctx: BuiltinContext, report: (detail: string) => void | Promise<void>): { stream: AsyncIterable<Uint8Array>; failed: () => boolean } {
  let failed = false
  const stream = (async function* () {
    try {
      for await (const chunk of ctx.stdin) {
        checkCancelled(ctx)
        yield chunk
      }
    } catch (error) {
      failed = true
      await report(strerror(error))
    }
  })()
  return { stream, failed: () => failed }
}

const stdinMessage = (program: string) => (detail: string) => `${program}: -: ${detail}\n`

/**
 * Opens a text tool's operands in order, reporting each unreadable one the
 * way coreutils does (`prog: name: strerror`) and carrying on. Returns the
 * inputs that opened and whether any failed.
 */
export async function openInputs(
  ctx: BuiltinContext,
  program: string,
  operands: readonly string[],
  message: (name: string, detail: string) => string = (name, detail) => `${program}: ${name}: ${detail}\n`,
  onStdin: (detail: string) => string = stdinMessage(program),
): Promise<{ inputs: Input[]; failed: boolean }> {
  const names = operands.length === 0 ? ['-'] : operands
  const inputs: Input[] = []
  let failed = false
  for (const name of names) {
    if (name === '-') {
      const guarded = guardedStdin(ctx, (detail) => ctx.stderr(onStdin(detail)))
      inputs.push({ name: 'standard input', stream: guarded.stream, readFailed: guarded.failed })
      continue
    }
    try {
      const bytes = await ctx.fs.readFile(name, { cwd: ctx.cwd })
      inputs.push({ name, stream: bytesStream(bytes), readFailed: () => false })
    } catch (error) {
      await ctx.stderr(message(name, strerror(error)))
      failed = true
    }
  }
  return { inputs, failed }
}

/** A stream that lazily opens a file, so a later operand's error appears after earlier output, as in coreutils. */
export function lazyInputs(
  ctx: BuiltinContext,
  program: string,
  operands: readonly string[],
  message: (name: string, detail: string) => string = (name, detail) => `${program}: ${name}: ${detail}\n`,
  onStdin: (detail: string) => string = stdinMessage(program),
): { name: string; open(): Promise<AsyncIterable<Uint8Array> | null>; readFailed: () => boolean }[] {
  const names = operands.length === 0 ? ['-'] : operands
  return names.map((name) => {
    let guarded: ReturnType<typeof guardedStdin> | null = null
    return {
      name: name === '-' ? 'standard input' : name,
      open: async () => {
        if (name === '-') {
          guarded = guardedStdin(ctx, (detail) => ctx.stderr(onStdin(detail)))
          return guarded.stream
        }
        try {
          return bytesStream(await ctx.fs.readFile(name, { cwd: ctx.cwd }))
        } catch (error) {
          await ctx.stderr(message(name, strerror(error)))
          return null
        }
      },
      readFailed: () => guarded?.failed() ?? false,
    }
  })
}
