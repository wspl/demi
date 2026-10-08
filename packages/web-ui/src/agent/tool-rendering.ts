import { nonEmptyString, numberOrNull, truncate } from '@demicodes/utils'

export const STANDARD_TOOL_NAMES = [
  'shell_exec',
  'shell_status',
  'yield'
] as const

export type StandardToolName = (typeof STANDARD_TOOL_NAMES)[number]
export type ControlToolName = Exclude<StandardToolName, 'shell_exec'>
export type ToolRenderKind = StandardToolName | 'generic'

const STANDARD_TOOL_NAME_SET = new Set<string>(STANDARD_TOOL_NAMES)

export function isStandardToolName(toolName: string): toolName is StandardToolName {
  return STANDARD_TOOL_NAME_SET.has(toolName)
}

export function shouldParsePartialToolInput(toolName: string): boolean {
  return isStandardToolName(toolName)
}

export function toolRenderKind(toolName: string): ToolRenderKind {
  return isStandardToolName(toolName) ? toolName : 'generic'
}

/**
 * A tool call's title in parts: its words, or, for a look or a wait without
 * a description, words before and after the command it names, which the
 * row shows as a reference (`runtime.md` § Rendering boundary).
 */
export type ToolTitle =
  | { kind: 'text', text: string }
  | { kind: 'reference', lead: string, commandId: string, trail: string }

export function standardToolTitleParts(
  toolName: StandardToolName,
  input: Record<string, unknown>
): ToolTitle {
  const description = nonEmptyString(input.description)
  if (description)
    return { kind: 'text', text: description }

  switch (toolName) {
    case 'shell_exec':
      return { kind: 'text', text: nonEmptyString(input.script) ?? 'Run shell command' }
    case 'shell_status': {
      const commandId = commandIdText(input.commandId)
      const writes = nonEmptyString(input.stdin) !== undefined
      if (commandId === undefined)
        return { kind: 'text', text: writes ? 'Send input' : 'Check command status' }
      return { kind: 'reference', lead: writes ? 'Send input to' : 'Check', commandId, trail: '' }
    }
    case 'yield': {
      const commandIds = Array.isArray(input.commandIds)
        ? input.commandIds.map(commandIdText).filter((id) => id !== undefined)
        : []
      const [first, ...others] = commandIds
      if (first !== undefined) {
        const trail = others.length > 0 ? `and ${others.length} more` : ''
        return { kind: 'reference', lead: 'Wait for', commandId: first, trail }
      }
      const duration = numberOrNull(input.durationMs)
      return {
        kind: 'text',
        text: duration === null ? 'Wait for wakeup' : `Wait ${Math.floor(duration)}ms`,
      }
    }
  }
}

/**
 * A tool call's title as plain text: a command it names by the title of the
 * call that started it, as `commandTitle` gives it, or as "command 17" when
 * no transcript the row sees holds that call.
 */
export function standardToolTitle(
  toolName: StandardToolName,
  input: Record<string, unknown>,
  commandTitle: (commandId: string) => string | undefined = () => undefined,
): string {
  const title = standardToolTitleParts(toolName, input)
  if (title.kind === 'text')
    return title.text
  const named = commandTitle(title.commandId) ?? unknownCommand(title.commandId)
  return [title.lead, named, title.trail].filter(Boolean).join(' ')
}

/** How a reference names a command no transcript the row sees holds: never a bare number. */
export function unknownCommand(commandId: string): string {
  return `command ${commandId}`
}

/** A command's number as a call names it: the model writes it as a number or its digits. */
function commandIdText(value: unknown): string | undefined {
  const number = numberOrNull(value)
  return number === null ? nonEmptyString(value) : String(number)
}

/**
 * The title of a call the model is still writing: its description once
 * written, and until then a title by tool (`runtime.md` § Calls being
 * written); a tool the runtime does not have is named.
 */
export function pendingCallTitle(call: { toolName: string, description: string | null }): string {
  const description = nonEmptyString(call.description)
  if (description)
    return description
  switch (toolRenderKind(call.toolName)) {
    case 'shell_exec':
      return 'Preparing a command…'
    case 'shell_status':
      return 'Checking a command…'
    case 'yield':
      return 'Waiting…'
    case 'generic':
      return call.toolName
  }
}

export function trimToolSummary(text: string, maxLength = 120): string {
  return truncate(text.replace(/\s+/g, ' ').trim(), maxLength, '...')
}
