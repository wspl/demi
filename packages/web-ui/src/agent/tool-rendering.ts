export const STANDARD_TOOL_NAMES = ['shell_exec', 'shell_status', 'shell_write', 'shell_abort', 'yield'] as const

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

export function standardToolTitle(toolName: StandardToolName, input: Record<string, unknown>): string {
  const description = optionalNonEmptyString(input.description)
  if (description) return description

  switch (toolName) {
    case 'shell_exec':
      return optionalNonEmptyString(input.script) ?? 'Run shell command'
    case 'shell_status': {
      const commandId = optionalNonEmptyString(input.commandId)
      return commandId ? `Check ${commandId}` : 'Check command status'
    }
    case 'shell_write': {
      const commandId = optionalNonEmptyString(input.commandId)
      return commandId ? `Send input to ${commandId}` : 'Send input'
    }
    case 'shell_abort': {
      const commandId = optionalNonEmptyString(input.commandId)
      return commandId ? `Stop ${commandId}` : 'Stop command'
    }
    case 'yield': {
      const duration = optionalFiniteNumber(input.durationMs)
      return duration === null ? 'Wait for wakeup' : `Wait ${Math.floor(duration)}ms`
    }
  }
}

export function trimToolSummary(text: string, maxLength = 120): string {
  const compact = text.replace(/\s+/g, ' ').trim()
  return compact.length > maxLength ? `${compact.slice(0, maxLength - 3)}...` : compact
}

function optionalNonEmptyString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value.trim() : null
}

function optionalFiniteNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}
