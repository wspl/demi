import { asRecord } from '@demicodes/utils'
import {
  listRegisteredCommandOperations,
  type BashEnvironment,
  type RegisteredCommandOperation,
} from '@demicodes/shell'
import { finishShellToolResult } from './tools'
import type { AgentTool } from './types'

/** Observation window for projected command invocations. Registered commands
 * execute in-process and complete quickly; long work belongs to shell_exec. */
const COMMAND_TOOL_TIMEOUT_MS = 60_000

export interface CommandToolProjectionOptions {
  /** Leaf paths to project (e.g. 'editor create'). Empty/absent = all leaves. */
  include?: string[]
  exclude?: string[]
}

/**
 * Projects registered command leaves onto the native tool surface: one
 * AgentTool per leaf, named by its path (editor create -> editor_create).
 * The tool renders the same shell invocation the model could type and runs it
 * through BashEnvironment.exec, so both projections share one implementation,
 * one audit trail, and one artifact store.
 */
export function createCommandProjectionTools<State = unknown>(
  environment: BashEnvironment,
  options: CommandToolProjectionOptions = {},
  reservedNames: ReadonlySet<string> = new Set(),
): AgentTool<State>[] {
  const operations = listRegisteredCommandOperations(environment.registeredCommands()).filter((operation) =>
    operationSelected(operation, options),
  )

  const seen = new Set<string>(reservedNames)
  return operations.map((operation) => {
    const name = operation.path.join('_')
    if (seen.has(name)) {
      throw new Error(`Command tool projection: tool name "${name}" conflicts with an existing tool`)
    }
    seen.add(name)
    return {
      name,
      description: operation.description,
      inputSchema: operation.inputSchema,
      invoke: async (ctx, input) => {
        const values = asRecord(input, `${name} input must be an object`)
        const result = await environment.exec({
          script: operation.renderScript(values),
          agentSessionId: ctx.agentSessionId,
          timeoutMs: COMMAND_TOOL_TIMEOUT_MS,
          signal: ctx.signal,
        })
        ctx.emitProgress(result)
        return finishShellToolResult(environment, result, ctx)
      },
    }
  })
}

function operationSelected(operation: RegisteredCommandOperation, options: CommandToolProjectionOptions): boolean {
  const path = operation.path.join(' ')
  if (options.exclude?.includes(path)) return false
  if (options.include && options.include.length > 0) return options.include.includes(path)
  return true
}
