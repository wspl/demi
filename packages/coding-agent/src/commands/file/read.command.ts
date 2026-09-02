import type { CommandContext, CommandResult } from '@demicodes/shell'

/** `demi file read <path>`: the file's bytes on stdout. */
export default async function read(ctx: CommandContext<{ path: string }>): Promise<CommandResult> {
  const { path } = ctx.args
  if (path.includes('\0')) {
    await ctx.stderr(`Path contains NUL byte: ${path}\n`)
    return { exitCode: 1 }
  }
  try {
    await ctx.stdout(await ctx.fs.readFile(path, { cwd: ctx.cwd }))
  } catch (error) {
    await ctx.stderr(`${error instanceof Error ? error.message : String(error)}\n`)
    return { exitCode: 1 }
  }
  return { exitCode: 0 }
}
