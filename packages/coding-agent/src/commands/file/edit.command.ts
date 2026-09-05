import type { CommandContext, CommandResult } from '@demicodes/shell'

interface EditArgs {
  path: string
  old: string
  new: string
  occurrence?: number
  context?: number
}

interface Match {
  index: number
  line: number
}

/** `demi file edit <path> --old … --new …`: one exact replacement, chosen by occurrence or nearest line. */
export default async function edit(ctx: CommandContext<EditArgs>): Promise<CommandResult> {
  const { path } = ctx.args
  if (path.includes('\0')) {
    await ctx.stderr(`Path contains NUL byte: ${path}\n`)
    return { exitCode: 1 }
  }
  let content: string
  try {
    content = new TextDecoder().decode(await ctx.fs.readFile(path, { cwd: ctx.cwd }))
  } catch (error) {
    await ctx.stderr(`${message(error)}\n`)
    return { exitCode: 1 }
  }

  const oldText = ctx.args.old
  if (oldText.length === 0) {
    await ctx.stderr('Old text must not be empty\n')
    return { exitCode: 1 }
  }
  const matches = findMatches(content, oldText)
  if (matches.length === 0) {
    await ctx.stderr(`No match found in ${path}\n`)
    return { exitCode: 1 }
  }

  const selection = chooseMatch(matches, ctx.args.occurrence, ctx.args.context)
  if (!selection.match) {
    await ctx.stderr(`${selection.reason} in ${path}: ${formatMatches(matches)}\n`)
    return { exitCode: 1 }
  }

  const match = selection.match
  const next = `${content.slice(0, match.index)}${ctx.args.new}${content.slice(match.index + oldText.length)}`
  try {
    await ctx.fs.writeFile(path, new TextEncoder().encode(next), { cwd: ctx.cwd })
  } catch (error) {
    await ctx.stderr(`${message(error)}\n`)
    return { exitCode: 1 }
  }
  await ctx.stdout(`Edited ${path}\n`)
  return { exitCode: 0 }
}

function findMatches(content: string, search: string): Match[] {
  const matches: Match[] = []
  let index = content.indexOf(search)
  while (index !== -1) {
    matches.push({ index, line: content.slice(0, index).split('\n').length })
    index = content.indexOf(search, index + search.length)
  }
  return matches
}

function chooseMatch(matches: Match[], occurrence: number | undefined, context: number | undefined): { match: Match | null; reason: string } {
  if (occurrence !== undefined) {
    const match = matches[occurrence - 1] ?? null
    return { match, reason: match ? '' : `Occurrence ${occurrence} is out of range` }
  }
  if (context !== undefined) {
    const ranked = matches
      .map((match) => ({ match, distance: Math.abs(match.line - context) }))
      .sort((a, b) => a.distance - b.distance)
    const nearest = ranked[0]
    if (!nearest) return { match: null, reason: 'No match found' }
    const tied = ranked.filter((entry) => entry.distance === nearest.distance)
    return tied.length === 1 ? { match: nearest.match, reason: '' } : { match: null, reason: `Context line ${context} is ambiguous` }
  }
  return matches.length === 1 ? { match: matches[0]!, reason: '' } : { match: null, reason: 'Multiple matches' }
}

function formatMatches(matches: Match[]): string {
  return matches.map((match, index) => `occurrence ${index + 1} at line ${match.line}`).join(', ')
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
