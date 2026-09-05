import type { Builtin } from './io'
import { resolvePath } from '../outside/namespace'
import { strerror } from './errors'

/** bash's `cd`: one path or none (the home). Errors carry bash's own prefix. */
export const cd: Builtin = async (ctx) => {
  const target = ctx.argv[0] ?? ctx.home
  if (ctx.argv.length > 1) {
    await ctx.stderr(`bash: line ${ctx.line}: cd: too many arguments\n`)
    return 1
  }
  const resolved = resolvePath(ctx.cwd, target)
  try {
    const stat = await ctx.fs.stat(resolved)
    if (!stat.isDirectory) {
      await ctx.stderr(`bash: line ${ctx.line}: cd: ${target}: Not a directory\n`)
      return 1
    }
  } catch (error) {
    await ctx.stderr(`bash: line ${ctx.line}: cd: ${target}: ${strerror(error)}\n`)
    return 1
  }
  ctx.shell.setCwd(resolved)
  return 0
}
