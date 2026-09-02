import type { BuiltinContext } from './io'
import { strerror } from './errors'
import { bytesStream } from '../exec/stream'

export interface Input {
  /** The operand as given; `standard input` for `-` or no operands. */
  name: string
  stream: AsyncIterable<Uint8Array>
}

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
): Promise<{ inputs: Input[]; failed: boolean }> {
  const names = operands.length === 0 ? ['-'] : operands
  const inputs: Input[] = []
  let failed = false
  for (const name of names) {
    if (name === '-') {
      inputs.push({ name: 'standard input', stream: ctx.stdin })
      continue
    }
    try {
      const bytes = await ctx.fs.readFile(name, { cwd: ctx.cwd })
      inputs.push({ name, stream: bytesStream(bytes) })
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
): { name: string; open(): Promise<AsyncIterable<Uint8Array> | null> }[] {
  const names = operands.length === 0 ? ['-'] : operands
  return names.map((name) => ({
    name: name === '-' ? 'standard input' : name,
    open: async () => {
      if (name === '-') return ctx.stdin
      try {
        return bytesStream(await ctx.fs.readFile(name, { cwd: ctx.cwd }))
      } catch (error) {
        await ctx.stderr(message(name, strerror(error)))
        return null
      }
    },
  }))
}
