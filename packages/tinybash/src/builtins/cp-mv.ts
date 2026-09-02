import type { Builtin, BuiltinContext } from './io'
import { type FlagSpec, parseFlags, has } from './flags'
import { quoteC, strerror, tryHelp } from './errors'
import { resolvePath } from '../outside/namespace'
import { isDirectory } from '../exec/fs'
import { basenamePath } from '@demicodes/utils'

export const cpSpec: FlagSpec = { switches: ['r'], valued: [] }

export const mvSpec: FlagSpec = { switches: [], valued: [] }

/** Operand checks shared by cp and mv; returns the sources and the destination. */
async function plan(ctx: BuiltinContext, program: string, operands: readonly string[]): Promise<{ sources: string[]; destination: string; intoDirectory: boolean } | number> {
  if (operands.length === 0) {
    await ctx.stderr(`${program}: missing file operand\n${tryHelp(program)}`)
    return 1
  }
  if (operands.length === 1) {
    await ctx.stderr(`${program}: missing destination file operand after ${quoteC(operands[0]!)}\n${tryHelp(program)}`)
    return 1
  }
  const destination = operands[operands.length - 1]!
  const sources = operands.slice(0, -1)
  const destinationIsDirectory = await isDirectory(ctx.fs, ctx.cwd, destination)
  if (sources.length > 1 && destinationIsDirectory !== true) {
    if (destinationIsDirectory === null) await ctx.stderr(`${program}: target ${quoteC(destination)}: No such file or directory\n`)
    else await ctx.stderr(`${program}: target ${quoteC(destination)}: Not a directory\n`)
    return 1
  }
  const intoDirectory = destinationIsDirectory === true || (destinationIsDirectory === null && destination.endsWith('/'))
  return { sources, destination, intoDirectory }
}

export const cp: Builtin = async (ctx) => {
  const flags = parseFlags('cp', ctx.argv, cpSpec, ctx.line)
  const recursive = has(flags, 'r')
  const planned = await plan(ctx, 'cp', flags.operands)
  if (typeof planned === 'number') return planned
  let status = 0
  for (const source of planned.sources) {
    const target = planned.intoDirectory ? `${planned.destination.replace(/\/+$/, '')}/${basenamePath(source)}` : planned.destination
    let stat
    try {
      stat = await ctx.fs.stat(source, { cwd: ctx.cwd })
    } catch (error) {
      await ctx.stderr(`cp: cannot stat ${quoteC(source)}: ${strerror(error)}\n`)
      status = 1
      continue
    }
    if (stat.isDirectory && !recursive) {
      await ctx.stderr(`cp: -r not specified; omitting directory ${quoteC(source)}\n`)
      status = 1
      continue
    }
    if (resolvePath(ctx.cwd, source) === resolvePath(ctx.cwd, target)) {
      await ctx.stderr(`cp: ${quoteC(source)} and ${quoteC(target)} are the same file\n`)
      status = 1
      continue
    }
    try {
      const parent = target.includes('/') ? target.slice(0, target.lastIndexOf('/')) || '/' : '.'
      if (!(await ctx.fs.exists(parent, { cwd: ctx.cwd }))) {
        await ctx.stderr(`cp: cannot create ${stat.isDirectory ? 'directory' : 'regular file'} ${quoteC(target)}: No such file or directory\n`)
        status = 1
        continue
      }
      await ctx.fs.cp(source, target, { cwd: ctx.cwd, recursive })
    } catch (error) {
      await ctx.stderr(`cp: cannot create regular file ${quoteC(target)}: ${strerror(error)}\n`)
      status = 1
    }
  }
  return status
}

export const mv: Builtin = async (ctx) => {
  const flags = parseFlags('mv', ctx.argv, mvSpec, ctx.line)
  const planned = await plan(ctx, 'mv', flags.operands)
  if (typeof planned === 'number') return planned
  let status = 0
  for (const source of planned.sources) {
    const target = planned.intoDirectory ? `${planned.destination.replace(/\/+$/, '')}/${basenamePath(source)}` : planned.destination
    try {
      await ctx.fs.lstat(source, { cwd: ctx.cwd })
    } catch (error) {
      await ctx.stderr(`mv: cannot stat ${quoteC(source)}: ${strerror(error)}\n`)
      status = 1
      continue
    }
    if (resolvePath(ctx.cwd, source) === resolvePath(ctx.cwd, target)) {
      await ctx.stderr(`mv: ${quoteC(source)} and ${quoteC(target)} are the same file\n`)
      status = 1
      continue
    }
    try {
      await ctx.fs.mv(source, target, { cwd: ctx.cwd })
    } catch (error) {
      await ctx.stderr(`mv: cannot move ${quoteC(source)} to ${quoteC(target)}: ${strerror(error)}\n`)
      status = 1
    }
  }
  return status
}
