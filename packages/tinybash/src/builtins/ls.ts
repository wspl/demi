import type { HostFileStat } from '@demicodes/shell'
import type { Builtin, BuiltinContext } from './io'
import { parseFlags, has } from './flags'
import { SPECS } from './table'
import { quoteC, strerror } from './errors'
import { latin1Bytes } from '../exec/stream'

interface Entry {
  name: string
  path: string
  stat: HostFileStat
  linkTarget: string | null
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

function byteCompare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

export function modeString(stat: HostFileStat): string {
  const type = stat.isSymbolicLink ? 'l' : stat.isDirectory ? 'd' : stat.isCharacterDevice ? 'c' : stat.isFIFO ? 'p' : '-'
  const m = stat.mode
  const bit = (mask: number, ch: string) => (m & mask ? ch : '-')
  const owner = `${bit(0o400, 'r')}${bit(0o200, 'w')}${m & 0o4000 ? (m & 0o100 ? 's' : 'S') : bit(0o100, 'x')}`
  const group = `${bit(0o40, 'r')}${bit(0o20, 'w')}${m & 0o2000 ? (m & 0o10 ? 's' : 'S') : bit(0o10, 'x')}`
  const other = `${bit(0o4, 'r')}${bit(0o2, 'w')}${m & 0o1000 ? (m & 0o1 ? 't' : 'T') : bit(0o1, 'x')}`
  return `${type}${owner}${group}${other}`
}

/** GNU's C-locale time column: `Mon dd HH:MM` within the last six months, `Mon dd  YYYY` otherwise. */
export function timeColumn(mtime: Date, now: Date): string {
  const sixMonths = 31556952 / 2 * 1000
  const recent = now.getTime() - sixMonths < mtime.getTime() && mtime.getTime() <= now.getTime()
  const month = MONTHS[mtime.getMonth()]!
  const day = String(mtime.getDate()).padStart(2)
  if (recent) {
    return `${month} ${day} ${String(mtime.getHours()).padStart(2, '0')}:${String(mtime.getMinutes()).padStart(2, '0')}`
  }
  return `${month} ${day}  ${mtime.getFullYear()}`
}

/** Allocated 1K blocks as ext4 reports them for ordinary files: 4K blocks, none for an empty file or a fast symlink. */
function blocks(stat: HostFileStat): number {
  if (stat.isSymbolicLink) return stat.size < 60 ? 0 : 4
  if (stat.isDirectory) return 4
  return Math.ceil(stat.size / 4096) * 4
}

export const ls: Builtin = async (ctx) => {
  const flags = parseFlags('ls', ctx.argv, SPECS.ls, ctx.line)
  const long = has(flags, 'l')
  const all = has(flags, 'a')
  const recursive = has(flags, 'R')
  const operands = flags.operands.length === 0 ? ['.'] : flags.operands
  const now = new Date()
  let status = 0
  const files: Entry[] = []
  const dirs: Entry[] = []
  for (const operand of operands) {
    let stat: HostFileStat
    let linkTarget: string | null = null
    try {
      stat = await ctx.fs.lstat(operand, { cwd: ctx.cwd })
      if (stat.isSymbolicLink) {
        linkTarget = await ctx.fs.readlink(operand, { cwd: ctx.cwd })
        // A symlink operand is followed unless listing it long-form.
        if (!long) {
          const target = await ctx.fs.stat(operand, { cwd: ctx.cwd }).catch(() => null)
          if (target !== null) stat = target
        }
      }
    } catch (error) {
      await ctx.stderr(`ls: cannot access ${quoteC(operand)}: ${strerror(error)}\n`)
      status = 2
      continue
    }
    const entry: Entry = { name: operand, path: operand, stat, linkTarget }
    if (stat.isDirectory && !(long && entry.linkTarget !== null)) dirs.push(entry)
    else files.push(entry)
  }
  files.sort((x, y) => byteCompare(x.name, y.name))
  dirs.sort((x, y) => byteCompare(x.name, y.name))
  const headers = recursive || operands.length > 1
  let printedSomething = false
  if (files.length > 0) {
    await printEntries(ctx, files, { long, now, total: false })
    printedSomething = true
  }
  const queue = [...dirs]
  while (queue.length > 0) {
    const dir = queue.shift()!
    if (headers) await ctx.stdout(latin1Bytes(`${printedSomething ? '\n' : ''}${dir.path}:\n`))
    printedSomething = true
    let names: string[]
    try {
      names = await ctx.fs.readdir(dir.path, { cwd: ctx.cwd })
    } catch (error) {
      await ctx.stderr(`ls: cannot open directory ${quoteC(dir.path)}: ${strerror(error)}\n`)
      status = 2
      continue
    }
    if (all) names.push('.', '..')
    else names = names.filter((name) => !name.startsWith('.'))
    names.sort(byteCompare)
    const entries: Entry[] = []
    for (const name of names) {
      const path = dir.path.endsWith('/') ? `${dir.path}${name}` : `${dir.path}/${name}`
      try {
        const stat = await ctx.fs.lstat(path, { cwd: ctx.cwd })
        const linkTarget = stat.isSymbolicLink ? await ctx.fs.readlink(path, { cwd: ctx.cwd }) : null
        entries.push({ name, path, stat, linkTarget })
      } catch (error) {
        await ctx.stderr(`ls: cannot access ${quoteC(path)}: ${strerror(error)}\n`)
        status = 2
      }
    }
    await printEntries(ctx, entries, { long, now, total: long })
    if (recursive) {
      const subdirs = entries.filter((entry) => entry.stat.isDirectory && entry.name !== '.' && entry.name !== '..')
      queue.unshift(...subdirs.map((entry) => ({ ...entry, name: entry.path })))
    }
  }
  return status
}

async function printEntries(ctx: BuiltinContext, entries: Entry[], options: { long: boolean; now: Date; total: boolean }): Promise<void> {
  if (!options.long) {
    for (const entry of entries) await ctx.stdout(latin1Bytes(`${entry.name}\n`))
    return
  }
  if (options.total) {
    await ctx.stdout(`total ${entries.reduce((sum, entry) => sum + blocks(entry.stat), 0)}\n`)
  }
  const rows = entries.map((entry) => ({
    mode: modeString(entry.stat),
    nlink: String(entry.stat.nlink ?? (entry.stat.isDirectory ? 2 : 1)),
    user: ctx.identity.user,
    group: ctx.identity.group,
    size: String(entry.stat.size),
    time: timeColumn(entry.stat.mtime, options.now),
    name: entry.linkTarget !== null ? `${entry.name} -> ${entry.linkTarget}` : entry.name,
  }))
  const width = (key: 'nlink' | 'user' | 'group' | 'size') => Math.max(...rows.map((row) => row[key].length))
  const w = { nlink: width('nlink'), user: width('user'), group: width('group'), size: width('size') }
  for (const row of rows) {
    await ctx.stdout(latin1Bytes(`${row.mode} ${row.nlink.padStart(w.nlink)} ${row.user.padEnd(w.user)} ${row.group.padEnd(w.group)} ${row.size.padStart(w.size)} ${row.time} ${row.name}\n`))
  }
}
