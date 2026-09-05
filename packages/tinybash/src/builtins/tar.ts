// `tar` (`tinybash.md` § Builtins): the wire format of a copy between hosts,
// admitted by structure. `c`, `x`, `t` with `-f` (default `-`), `-C`, `-v`,
// `-z` over the platform's gzip stream, and `--strip-components=N` on `x`.
// Archives are written in GNU format with members in name order and the
// session user as owner; extraction follows GNU's own path rules inside
// the filesystem it is given, keeps mode and mtime, and refuses links —
// the hostless tree holds none — as GNU does on a filesystem that
// refuses them: the entry fails, the rest is extracted, the exit is 2.
import type { Builtin, BuiltinContext } from './io'
import { checkCancelled } from './io'
import { outside } from '../outside/reasons'
import { quoteC, strerror } from './errors'
import { resolvePath } from '../outside/namespace'
import { bytesStream } from '@demicodes/utils'
import { BlockReader, archiveEnd, decodeHeader, decodePax, encodeHeader, modeString, padToBlock, type TarEntryKind, type TarHeader } from './tar-format'

export interface TarArgs {
  mode: 'c' | 'x' | 't' | null
  /** How many mode letters were given; GNU refuses more than one. */
  modes: number
  file: string
  verbose: boolean
  gzip: boolean
  strip: number
  /** Every `-C`, for the parse-time path check. */
  dirs: string[]
  /** The `-C` in force at the end: where `x` extracts. */
  extractDir: string
  /** `c`: what to archive, each with the `-C` in force before it; `x` and `t`: member names to select. */
  members: { name: string; dir: string }[]
}

const LETTERS = 'cxtfvzC'

/**
 * GNU's two spellings: old style (`tar czf file dir`: the first word's letters
 * are options, `f` and `C` taking the words that follow in order) and
 * dashed (`-czf file`, `-C dir`, permuted with operands). Flags outside the
 * whitelist are outside the subset.
 */
export function parseTarArgs(argv: readonly string[], line: number): TarArgs {
  const args: TarArgs = { mode: null, modes: 0, file: '-', verbose: false, gzip: false, strip: 0, dirs: [], extractDir: '.', members: [] }
  const words = [...argv]
  let currentDir = '.'
  const takeValue = (letter: string): string => {
    const next = words.shift()
    if (next === undefined) outside({ kind: 'flag', program: 'tar', flag: `-${letter} without a value`, line })
    return next
  }
  const apply = (letter: string, valueOf: () => string): void => {
    switch (letter) {
      case 'c':
      case 'x':
      case 't':
        args.mode = letter
        args.modes += 1
        return
      case 'f':
        args.file = valueOf()
        return
      case 'C':
        currentDir = under(currentDir, valueOf())
        args.dirs.push(currentDir)
        args.extractDir = currentDir
        return
      case 'v':
        args.verbose = true
        return
      case 'z':
        args.gzip = true
        return
      default:
        outside({ kind: 'flag', program: 'tar', flag: `-${letter}`, line })
    }
  }
  const first = words[0]
  if (first !== undefined && !first.startsWith('-')) {
    words.shift()
    const pending: string[] = []
    for (const letter of first) {
      if (!LETTERS.includes(letter)) outside({ kind: 'flag', program: 'tar', flag: `-${letter}`, line })
      if (letter === 'f' || letter === 'C') pending.push(letter)
      else apply(letter, () => '')
    }
    for (const letter of pending) apply(letter, () => takeValue(letter))
  }
  while (words.length > 0) {
    const word = words.shift()!
    if (word === '--') {
      for (const rest of words) args.members.push({ name: rest, dir: currentDir })
      break
    }
    if (word.startsWith('--')) {
      const [name, attached] = word.slice(2).split(/=(.*)/s, 2) as [string, string | undefined]
      if (name !== 'strip-components') outside({ kind: 'flag', program: 'tar', flag: word, line })
      const text = attached ?? takeValue('-strip-components')
      const count = /^\d+$/.test(text) ? Number(text) : Number.NaN
      if (!Number.isFinite(count)) outside({ kind: 'flag', program: 'tar', flag: `--strip-components=${text}`, line })
      args.strip = count
      continue
    }
    if (word.startsWith('-') && word !== '-') {
      const letters = word.slice(1)
      for (let i = 0; i < letters.length; i += 1) {
        const letter = letters[i]!
        if (letter === 'f' || letter === 'C') {
          const attached = letters.slice(i + 1)
          apply(letter, () => (attached.length > 0 ? attached : takeValue(letter)))
          break
        }
        apply(letter, () => '')
      }
      continue
    }
    args.members.push({ name: word, dir: currentDir })
  }
  return args
}

/** The paths the parse-time check sees: every `-C`, the archive file, and for `c` each operand under its `-C`. */
export function tarPaths(argv: readonly string[], line: number): string[] {
  const args = parseTarArgs(argv, line)
  const paths = [...args.dirs]
  if (args.file !== '-') paths.push(args.file)
  if (args.mode === 'c') {
    for (const member of args.members) paths.push(under(member.dir, member.name))
  }
  return paths
}

/** `name` under the `-C` directory in force: as given when absolute or when no `-C` applies. */
function under(dir: string, name: string): string {
  if (dir === '.' || name.startsWith('/')) return name
  return `${dir.replace(/\/+$/, '')}/${name}`
}

const USAGE = "Try 'tar --help' or 'tar --usage' for more information.\n"
const FAILURE = 'tar: Exiting with failure status due to previous errors\n'
/** GNU applies the process umask to extracted modes unless `-p`; the hostless umask is 022. */
const UMASK = 0o022
/** The owner every hostless file has; the names come from the session identity. */
const OWNER = { uid: 1000, gid: 1000 }

export const tar: Builtin = async (ctx) => {
  const args = parseTarArgs(ctx.argv, ctx.line)
  if (args.modes > 1) {
    await ctx.stderr(`tar: You may not specify more than one '-Acdtrux', '--delete' or  '--test-label' option\n${USAGE}`)
    return 2
  }
  if (args.mode === null) {
    await ctx.stderr(`tar: You must specify one of the '-Acdtrux', '--delete' or '--test-label' options\n${USAGE}`)
    return 2
  }
  switch (args.mode) {
    case 'c':
      return create(ctx, args)
    case 'x':
      return extract(ctx, args)
    case 't':
      return list(ctx, args)
  }
}

// --- create ------------------------------------------------------------------------

async function create(ctx: BuiltinContext, args: TarArgs): Promise<number> {
  if (args.members.length === 0) {
    await ctx.stderr(`tar: Cowardly refusing to create an empty archive\n${USAGE}`)
    return 2
  }
  const sink = await openSink(ctx, args)
  const verbose = args.verbose ? (args.file === '-' ? ctx.stderr : ctx.stdout) : null
  let written = 0
  let failed = false
  let leadingSlashWarned = false
  const emit = async (bytes: Uint8Array) => {
    written += bytes.byteLength
    await sink.write(bytes)
  }
  const member = async (name: string, dir: string): Promise<void> => {
    checkCancelled(ctx)
    const base = dir === '.' ? ctx.cwd : resolvePath(ctx.cwd, dir)
    let stat
    try {
      stat = await ctx.fs.lstat(name, { cwd: base })
    } catch (error) {
      await ctx.stderr(`tar: ${name}: Cannot stat: ${strerror(error)}\n`)
      failed = true
      return
    }
    let archiveName = name
    if (archiveName.startsWith('/')) {
      if (!leadingSlashWarned) {
        await ctx.stderr("tar: Removing leading `/' from member names\n")
        leadingSlashWarned = true
      }
      archiveName = archiveName.replace(/^\/+/, '')
    }
    const kind: TarEntryKind = stat.isDirectory ? 'directory' : stat.isSymbolicLink ? 'symlink' : 'file'
    if (kind === 'directory' && !archiveName.endsWith('/')) archiveName += '/'
    if (verbose) await verbose(`${archiveName}\n`)
    const header: Omit<TarHeader, 'typeflag'> = {
      name: archiveName,
      kind,
      mode: stat.mode & 0o7777,
      uid: OWNER.uid,
      gid: OWNER.gid,
      size: kind === 'file' ? stat.size : 0,
      mtime: stat.mtime,
      linkName: kind === 'symlink' ? await ctx.fs.readlink(name, { cwd: base }) : '',
      uname: ctx.identity.user,
      gname: ctx.identity.group,
    }
    if (kind === 'file') {
      let data: Uint8Array
      try {
        data = await ctx.fs.readFile(name, { cwd: base })
      } catch (error) {
        await ctx.stderr(`tar: ${name}: Cannot open: ${strerror(error)}\n`)
        failed = true
        return
      }
      header.size = data.byteLength
      for (const block of encodeHeader(header)) await emit(block)
      await emit(data)
      await emit(padToBlock(data.byteLength))
      return
    }
    for (const block of encodeHeader(header)) await emit(block)
    if (kind !== 'directory') return
    let children: string[]
    try {
      children = (await ctx.fs.readdir(name, { cwd: base })).sort()
    } catch (error) {
      await ctx.stderr(`tar: ${name}: Cannot savedir: ${strerror(error)}\n`)
      failed = true
      return
    }
    for (const child of children) await member(`${name.replace(/\/+$/, '')}/${child}`, dir)
  }
  try {
    for (const entry of args.members) await member(entry.name, entry.dir)
    await emit(archiveEnd(written))
    await sink.end()
  } catch (error) {
    await sink.end().catch(() => {})
    throw error
  }
  if (failed) await ctx.stderr(FAILURE)
  return failed ? 2 : 0
}

interface Sink {
  write(bytes: Uint8Array): Promise<void>
  end(): Promise<void>
}

/** Where the archive goes: stdout or `-f`'s file, through gzip when `-z`. */
async function openSink(ctx: BuiltinContext, args: TarArgs): Promise<Sink> {
  let raw: Sink
  if (args.file === '-') {
    raw = { write: async (bytes) => void (await ctx.stdout(bytes)), end: async () => {} }
  } else {
    await ctx.fs.writeFile(args.file, new Uint8Array(0), { cwd: ctx.cwd })
    raw = { write: (bytes) => ctx.fs.appendFile(args.file, bytes, { cwd: ctx.cwd }), end: async () => {} }
  }
  if (!args.gzip) return raw
  const compressor = new CompressionStream('gzip')
  const writer = compressor.writable.getWriter()
  const drained = (async () => {
    for await (const chunk of compressor.readable as unknown as AsyncIterable<Uint8Array>) await raw.write(chunk)
  })()
  return {
    write: (bytes) => writer.write(bytes as Uint8Array<ArrayBuffer>),
    end: async () => {
      await writer.close()
      await drained
    },
  }
}

// --- read side ---------------------------------------------------------------------

/** Where the archive comes from: stdin or `-f`'s file, through gunzip when `-z`. */
async function openSource(ctx: BuiltinContext, args: TarArgs): Promise<AsyncIterable<Uint8Array> | null> {
  let raw: AsyncIterable<Uint8Array>
  if (args.file === '-') raw = ctx.stdin
  else {
    try {
      raw = bytesStream(await ctx.fs.readFile(args.file, { cwd: ctx.cwd }))
    } catch (error) {
      await ctx.stderr(`tar: ${args.file}: Cannot open: ${strerror(error)}\ntar: Error is not recoverable: exiting now\n`)
      return null
    }
  }
  if (!args.gzip) return raw
  const iterator = raw[Symbol.asyncIterator]()
  const body = new ReadableStream<Uint8Array>({
    pull: async (controller) => {
      const next = await iterator.next()
      if (next.done) controller.close()
      else controller.enqueue(next.value)
    },
    cancel: () => void iterator.return?.(),
  })
  return body.pipeThrough(new DecompressionStream('gzip') as unknown as ReadableWritablePair<Uint8Array, Uint8Array>) as unknown as AsyncIterable<Uint8Array>
}

/** One member with GNU's long-name and pax overrides applied, or `null` at the archive's end. */
async function nextMember(reader: BlockReader): Promise<TarHeader | null> {
  let longName: string | null = null
  let longLink: string | null = null
  let pax: ReturnType<typeof decodePax> = {}
  for (;;) {
    const block = await reader.next()
    if (block === null) return null
    const header = decodeHeader(block)
    if (header === null) return null
    switch (header.typeflag) {
      case 'L':
        longName = trimNul(await reader.data(header.size))
        continue
      case 'K':
        longLink = trimNul(await reader.data(header.size))
        continue
      case 'x':
        pax = { ...pax, ...decodePax(await reader.data(header.size)) }
        continue
      case 'g':
        await reader.data(header.size)
        continue
    }
    return {
      ...header,
      name: pax.name ?? longName ?? header.name,
      linkName: pax.linkName ?? longLink ?? header.linkName,
      size: pax.size ?? header.size,
      mtime: pax.mtime ?? header.mtime,
    }
  }
}

function trimNul(bytes: Uint8Array): string {
  let end = bytes.byteLength
  while (end > 0 && bytes[end - 1] === 0) end -= 1
  return new TextDecoder().decode(bytes.subarray(0, end))
}

/** Reads the archive member by member; a malformed or truncated archive ends it with GNU's line and exit 2. */
async function readMembers(ctx: BuiltinContext, args: TarArgs, each: (header: TarHeader, reader: BlockReader, select: (name: string) => boolean) => Promise<void>): Promise<number> {
  const source = await openSource(ctx, args)
  if (source === null) return 2
  const reader = new BlockReader(source)
  const missing = new Set(args.members)
  const select = (name: string) => selected(args, name, missing)
  try {
    for (;;) {
      checkCancelled(ctx)
      const header = await nextMember(reader)
      if (header === null) break
      await each(header, reader, select)
    }
    if (missing.size > 0) {
      for (const member of missing) await ctx.stderr(`tar: ${member.name}: Not found in archive\n`)
      await ctx.stderr(FAILURE)
      return 2
    }
    return 0
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (message === 'This does not look like a tar archive' || message === 'Unexpected EOF in archive') {
      await ctx.stderr(`tar: ${message}\n${FAILURE}`)
      return 2
    }
    if (args.gzip) {
      await ctx.stderr(`\ngzip: stdin: not in gzip format\ntar: Child returned status 1\ntar: Error is not recoverable: exiting now\n`)
      return 2
    }
    throw error
  } finally {
    await reader.close().catch(() => {})
  }
}

/** Whether `x`/`t` operands select this member: the member or anything under a named directory. */
function selected(args: TarArgs, name: string, missing: Set<TarArgs['members'][number]>): boolean {
  if (args.members.length === 0) return true
  const plain = name.replace(/\/+$/, '')
  let matched = false
  for (const member of args.members) {
    const wanted = member.name.replace(/\/+$/, '')
    if (plain === wanted || plain.startsWith(`${wanted}/`)) {
      missing.delete(member)
      matched = true
    }
  }
  return matched
}

// --- extract -----------------------------------------------------------------------

async function extract(ctx: BuiltinContext, args: TarArgs): Promise<number> {
  let status = 0
  let leadingSlashWarned = false
  const strippedPrefixes = new Set<string>()
  const directories: { path: string; mode: number; mtime: Date }[] = []
  const fail = async (line: string) => {
    await ctx.stderr(line)
    status = 2
  }
  const outcome = await readMembers(ctx, args, async (header, reader, select) => {
    const skipData = () => reader.data(header.kind === 'file' || header.kind === 'other' ? header.size : 0)
    let name = header.name
    if (name.startsWith('/')) {
      if (!leadingSlashWarned) {
        await ctx.stderr("tar: Removing leading `/' from member names\n")
        leadingSlashWarned = true
      }
      name = name.replace(/^\/+/, '')
    }
    if (!select(name)) {
      await skipData()
      return
    }
    const components = name.split('/').filter((part) => part !== '' && part !== '.')
    if (components.includes('..')) {
      // GNU names the prefix through the last `..` it strips, once per prefix, then refuses the member.
      await skipData()
      const prefix = name.slice(0, name.lastIndexOf('..') + 3)
      if (!strippedPrefixes.has(prefix)) {
        strippedPrefixes.add(prefix)
        await ctx.stderr(`tar: Removing leading \`${prefix}' from member names\n`)
      }
      await fail(`tar: ${name}: Member name contains '..'\n`)
      return
    }
    const kept = components.slice(args.strip)
    if (kept.length === 0) {
      await skipData()
      return
    }
    const target = under(args.extractDir, kept.join('/'))
    if (args.verbose) await ctx.stdout(`${name}\n`)
    const parent = kept.length > 1 ? under(args.extractDir, kept.slice(0, -1).join('/')) : args.extractDir
    try {
      switch (header.kind) {
        case 'directory':
          await ctx.fs.mkdir(target, { cwd: ctx.cwd, recursive: true })
          directories.push({ path: target, mode: header.mode & ~UMASK, mtime: header.mtime })
          return
        case 'file': {
          const data = await reader.data(header.size)
          if (parent !== '.') await ctx.fs.mkdir(parent, { cwd: ctx.cwd, recursive: true })
          await ctx.fs.writeFile(target, data, { cwd: ctx.cwd })
          await ctx.fs.chmod(target, header.mode & ~UMASK, { cwd: ctx.cwd })
          await ctx.fs.utimes(target, header.mtime, header.mtime, { cwd: ctx.cwd })
          return
        }
        case 'symlink':
          await fail(`tar: ${name}: Cannot create symlink to ${quoteC(header.linkName)}: Operation not permitted\n`)
          return
        case 'hardlink':
          await fail(`tar: ${name}: Cannot hard link to ${quoteC(header.linkName)}: Operation not permitted\n`)
          return
        case 'other':
          await skipData()
          await fail(`tar: ${name}: Cannot mknod: Operation not permitted\n`)
          return
      }
    } catch (error) {
      await fail(`tar: ${name}: Cannot open: ${strerror(error)}\n`)
    }
  })
  // Directory modes and times last, deepest first, as GNU defers them: extracting into a directory would touch them.
  for (const directory of directories.reverse()) {
    try {
      await ctx.fs.chmod(directory.path, directory.mode, { cwd: ctx.cwd })
      await ctx.fs.utimes(directory.path, directory.mtime, directory.mtime, { cwd: ctx.cwd })
    } catch (error) {
      await fail(`tar: ${directory.path}: Cannot utime: ${strerror(error)}\n`)
    }
  }
  if (status !== 0 && outcome === 0) await ctx.stderr(FAILURE)
  return status || outcome
}

// --- list --------------------------------------------------------------------------

async function list(ctx: BuiltinContext, args: TarArgs): Promise<number> {
  // GNU widens the owner and size columns as it goes, never narrowing them.
  let ugswidth = 19
  return readMembers(ctx, args, async (header, reader, select) => {
    if (header.kind === 'file' || header.kind === 'other') await reader.data(header.size)
    if (!select(header.name)) return
    if (!args.verbose) {
      await ctx.stdout(`${header.name}\n`)
      return
    }
    const owner = `${header.uname || String(header.uid)}/${header.gname || String(header.gid)}`
    const size = String(header.size)
    if (owner.length + 1 + size.length > ugswidth) ugswidth = owner.length + 1 + size.length
    const link = header.kind === 'symlink' ? ` -> ${header.linkName}` : header.kind === 'hardlink' ? ` link to ${header.linkName}` : ''
    await ctx.stdout(`${modeString(header.kind, header.mode)} ${owner}${' '.repeat(ugswidth - owner.length - size.length)}${size} ${timestamp(header.mtime)} ${header.name}${link}\n`)
  })
}

/** `YYYY-MM-DD HH:MM` in UTC, the corpus's clock. */
function timestamp(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}`
}
