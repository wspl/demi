import type { CommandContext, CommandResult } from '@demicodes/shell'

/** `demi file create <path>` with the content on stdin: a new file, never an overwrite. */
export default async function create(ctx: CommandContext<{ path: string; content: string }>): Promise<CommandResult> {
  const { path, content } = ctx.args
  if (path.includes('\0')) {
    await ctx.stderr(`Path contains NUL byte: ${path}\n`)
    return { exitCode: 1 }
  }
  if (await ctx.fs.exists(path, { cwd: ctx.cwd })) {
    await ctx.stderr(`File already exists: ${path}\n`)
    return { exitCode: 1 }
  }
  try {
    await ctx.fs.writeFile(path, new TextEncoder().encode(content), { cwd: ctx.cwd, createParents: true })
  } catch (error) {
    await ctx.stderr(`${error instanceof Error ? error.message : String(error)}\n`)
    return { exitCode: 1 }
  }
  await ctx.stdout(`Created ${path}\n`)
  return { exitCode: 0 }
}
