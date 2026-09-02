import type { CommandContext, CommandResult } from '@demicodes/shell'

/**
 * A third party's own `runtime` command: `scout hello <path>` writes a
 * greeting file. It runs wherever the command is invoked — here inside the
 * embedder's process against the embedder's Host — and imports nothing but
 * types.
 */
export default async function hello(ctx: CommandContext<{ path: string; name?: string }>): Promise<CommandResult> {
  const text = `hello, ${ctx.args.name ?? 'world'}\n`
  await ctx.fs.writeFile(ctx.args.path, new TextEncoder().encode(text), { cwd: ctx.cwd, createParents: true })
  await ctx.stdout(`wrote ${ctx.args.path}\n`)
  return { exitCode: 0 }
}
