export function extractSimpleBackgroundCommand(statementInput: unknown): { command: string; args: string[] } | null {
  const statement = statementInput as {
    background?: boolean
    operators?: unknown[]
    pipelines?: Array<{
      commands?: unknown[]
    }>
  }
  if (!statement.background || (statement.operators?.length ?? 0) > 0 || statement.pipelines?.length !== 1) return null
  const pipeline = statement.pipelines[0] as { commands?: Array<{ type?: string; name?: unknown; args?: unknown[]; assignments?: unknown[]; redirections?: unknown[] }> }
  if (pipeline.commands?.length !== 1) return null
  const commandNode = pipeline.commands[0]
  if (commandNode.type !== 'SimpleCommand' || (commandNode.assignments?.length ?? 0) > 0 || (commandNode.redirections?.length ?? 0) > 0) return null
  const command = wordToBackgroundText(commandNode.name)
  if (!command) return null
  const args: string[] = []
  for (const arg of commandNode.args ?? []) {
    const value = wordToBackgroundText(arg)
    if (value === null) return null
    args.push(value)
  }
  return { command, args }
}

export function formatCommandDisplay(command: string, args: string[]): string {
  return [command, ...args].join(' ')
}

function wordToBackgroundText(word: unknown): string | null {
  const node = word as { type?: string; parts?: unknown[] } | null
  if (node?.type !== 'Word' || !node.parts) return null
  let text = ''
  for (const part of node.parts) {
    const value = wordPartToBackgroundText(part)
    if (value === null) return null
    text += value
  }
  return text
}

function wordPartToBackgroundText(part: unknown): string | null {
  const node = part as { type?: string; value?: string; parts?: unknown[]; pattern?: string; user?: string | null }
  switch (node.type) {
    case 'Literal':
    case 'SingleQuoted':
    case 'Escaped':
      return node.value ?? ''
    case 'Glob':
      return node.pattern ?? ''
    case 'TildeExpansion':
      return node.user === null ? '~' : `~${node.user ?? ''}`
    case 'DoubleQuoted': {
      let text = ''
      for (const child of node.parts ?? []) {
        const value = wordPartToBackgroundText(child)
        if (value === null) return null
        text += value
      }
      return text
    }
    default:
      return null
  }
}
