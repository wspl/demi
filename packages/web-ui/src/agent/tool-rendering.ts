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

export function standardToolTitle(
  toolName: StandardToolName,
  input: Record<string, unknown>
): string {
  const description = nonEmptyString(input.description)
  if (description)
    return description

  switch (toolName) {
    case 'shell_exec':
      return nonEmptyString(input.script) ?? 'Run shell command'
    case 'shell_status': {
      const commandId = commandIdText(input.commandId)
      if (nonEmptyString(input.stdin))
        return commandId ? `Send input to ${commandId}` : 'Send input'
      return commandId ? `Check ${commandId}` : 'Check command status'
    }
    case 'yield': {
      const commandIds = Array.isArray(input.commandIds)
        ? input.commandIds.map(commandIdText).filter((id) => id !== undefined)
        : []
      if (commandIds.length > 0)
        return `Wait for ${commandIds.join(', ')}`
      const duration = numberOrNull(input.durationMs)
      return duration === null
        ? 'Wait for wakeup'
        : `Wait ${Math.floor(duration)}ms`
    }
  }
}

/** A command's number as a call names it: the model writes it as a number or its digits. */
function commandIdText(value: unknown): string | undefined {
  const number = numberOrNull(value)
  return number === null ? nonEmptyString(value) : String(number)
}

export function trimToolSummary(text: string, maxLength = 120): string {
  return truncate(text.replace(/\s+/g, ' ').trim(), maxLength, '...')
}
