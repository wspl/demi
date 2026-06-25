export const STANDARD_TOOL_NAMES = ['shell_exec', 'shell_status', 'shell_write', 'shell_abort', 'yield'] as const

export type StandardToolName = (typeof STANDARD_TOOL_NAMES)[number]
export type ControlToolName = Exclude<StandardToolName, 'shell_exec'>
export type ToolRenderKind = StandardToolName | 'generic'

export interface ToolDisplayRow {
  label: string
  value: string
  monospace?: boolean
}

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

export function standardToolRows(toolName: ControlToolName, input: Record<string, unknown>): ToolDisplayRow[] {
  switch (toolName) {
    case 'shell_status':
      return compactRows([
        stringRow('commandId', input.commandId, true),
        numberRow('stdoutOffset', input.stdoutOffset),
        numberRow('stderrOffset', input.stderrOffset),
        numberRow('maxOutputBytes', input.maxOutputBytes),
      ])
    case 'shell_write':
      return compactRows([
        stringRow('commandId', input.commandId, true),
        stringRow('stdin', input.stdin, true),
        numberRow('maxOutputBytes', input.maxOutputBytes),
      ])
    case 'shell_abort':
      return compactRows([stringRow('commandId', input.commandId, true), numberRow('maxOutputBytes', input.maxOutputBytes)])
    case 'yield':
      return compactRows([numberRow('durationMs', input.durationMs)])
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

function stringRow(label: string, value: unknown, monospace = false): ToolDisplayRow | null {
  const text = optionalNonEmptyString(value)
  return text ? { label, value: trimToolSummary(text, 200), monospace } : null
}

function numberRow(label: string, value: unknown): ToolDisplayRow | null {
  const number = optionalFiniteNumber(value)
  return number === null ? null : { label, value: String(Math.floor(number)), monospace: true }
}

function compactRows(rows: Array<ToolDisplayRow | null>): ToolDisplayRow[] {
  return rows.filter((row): row is ToolDisplayRow => row !== null)
}
