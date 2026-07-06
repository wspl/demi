import { z } from 'zod'
import {
  isCommandGroup,
  type CommandInputSpec,
  type CommandSpec,
  type CommandSubcommandSpec,
} from './command'
import type { CommandRegistry } from './command'

/**
 * A registered command leaf described as a structured operation: the second
 * projection of a CommandSpec. The shell invocation (argv + heredoc) and this
 * form share one implementation — `renderScript` produces a script that runs
 * through the normal BashEnvironment path, so audit, command records, and
 * `/@/commands` artifacts behave identically for both projections.
 */
export interface RegisteredCommandOperation {
  /** Path segments from root command to leaf, e.g. ['editor', 'create']. */
  path: string[]
  description: string
  /** JSON Schema for the operation input, derived from the leaf's zod spec. */
  inputSchema: Record<string, unknown>
  /**
   * Renders a shell script invoking this operation with the given input.
   * The stdin field (if any) is delivered as a quoted heredoc, which — like
   * any heredoc — normalizes the body to end with a newline.
   */
  renderScript(values: Record<string, unknown>): string
}

export function listRegisteredCommandOperations(
  commands: CommandRegistry | CommandSpec[],
): RegisteredCommandOperation[] {
  const specs = Array.isArray(commands) ? commands : commands.list()
  const operations: RegisteredCommandOperation[] = []
  for (const spec of specs) collectOperations(spec, [spec.name], operations)
  return operations
}

function collectOperations(
  node: CommandSpec,
  path: string[],
  operations: RegisteredCommandOperation[],
): void {
  for (const child of node.subcommands) {
    if (isCommandGroup(child)) {
      collectOperations(child, [...path, child.name], operations)
      continue
    }
    operations.push(operationFromLeaf(child, [...path, child.name]))
  }
}

function operationFromLeaf(leaf: CommandSubcommandSpec, path: string[]): RegisteredCommandOperation {
  const parts = [leaf.summary.trim()]
  if (leaf.effects) parts.push(`Effects: ${leaf.effects}`)
  return {
    path,
    description: parts.join(' '),
    inputSchema: inputSchemaFromSpec(leaf.input ?? {}),
    renderScript: (values) => renderInvocationScript(leaf, path, values),
  }
}

function inputSchemaFromSpec(input: CommandInputSpec): Record<string, unknown> {
  const schema = z.toJSONSchema(z.object(input)) as Record<string, unknown>
  // Tool schemas are standalone; the meta-schema pointer is noise for models.
  delete schema.$schema
  return schema
}

function renderInvocationScript(
  leaf: CommandSubcommandSpec,
  path: string[],
  values: Record<string, unknown>,
): string {
  const tokens = [...path]

  // Every field goes through its --flag form (the parser accepts flags for
  // positional-declared fields too), which stays unambiguous for any value.
  for (const field of Object.keys(leaf.input ?? {})) {
    if (field === leaf.stdinField) continue
    const value = values[field]
    if (value === undefined || value === null) continue
    for (const item of Array.isArray(value) ? value : [value]) {
      tokens.push(`--${field}`, shellQuote(scalarText(item)))
    }
  }

  const script = tokens.join(' ')
  if (!leaf.stdinField) return script
  const stdinValue = values[leaf.stdinField]
  const body = stdinValue === undefined || stdinValue === null ? '' : scalarText(stdinValue)
  const delimiter = heredocDelimiter(body)
  return `${script} <<'${delimiter}'\n${body}\n${delimiter}`
}

function scalarText(value: unknown): string {
  if (typeof value === 'string') return value
  if (typeof value === 'number' || typeof value === 'boolean') return String(value)
  return JSON.stringify(value)
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`
}

function heredocDelimiter(body: string): string {
  let delimiter = 'DEMI_STDIN'
  const lines = new Set(body.split('\n'))
  while (lines.has(delimiter)) delimiter += '_X'
  return delimiter
}
