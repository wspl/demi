import type { AgentDefinition } from '@demi/base-agent'
import {
  CommandRegistry,
  createShellSessionTools,
  type BashEnvironment,
  type CommandSpec,
  type Host,
} from '@demi/shell'
import { createEditorCommand } from './editor-command'
import { createFileReferenceResolver } from './reference-resolver'
import { createTodoCommand } from './todo-command'

export type CodingState = Record<string, never>

export interface CodingAgentOptions {
  environment: BashEnvironment
  editorHost?: Host
  referenceHost?: Host
  commands?: CommandSpec[]
}

export interface CodingCommandRegistryOptions {
  editorHost?: Host
  commands?: CommandSpec[]
}

export function createCodingCommandRegistry(options: CodingCommandRegistryOptions = {}): CommandRegistry {
  const registry = new CommandRegistry()
  const commands = options.commands ?? [
    ...(options.editorHost ? [createEditorCommand(options.editorHost)] : []),
    createTodoCommand(),
  ]
  for (const command of commands) registry.register(command)
  return registry
}

export function createCodingAgentDefinition(options: CodingAgentOptions): AgentDefinition<CodingState> {
  const commands = options.commands ?? defaultCodingCommands(options.editorHost ?? options.environment.workspaceHost())
  const referenceHost = options.referenceHost ?? options.editorHost ?? options.environment.workspaceHost()
  for (const command of commands) options.environment.registerCommand(command)
  const resolveReferences = createFileReferenceResolver<CodingState>(referenceHost)
  return {
    name: 'coding',
    initialState: () => ({}),
    systemPrompt: () => {
      const commandPrompt = renderCommandList(commands)
      const sections = [
        'You are a coding agent. Use shell session tools to inspect, edit, test, and verify the workspace.',
        'Prefer registered commands for agent-specific state and audited workflows. Use normal system commands for ordinary shell work.',
        'File references attached by the client are expanded before provider calls.',
      ]
      if (commandPrompt.trim()) sections.push(`Registered commands:\n\n${commandPrompt}`)
      return sections.join('\n\n')
    },
    resolveReferences,
    tools: () => createShellSessionTools(options.environment),
    commands: () => [...commands],
    dispose: () => options.environment.disposeAllSessions(),
  }
}

function defaultCodingCommands(editorHost: Host): CommandSpec[] {
  return [createEditorCommand(editorHost), createTodoCommand()]
}

function renderCommandList(commands: CommandSpec[]): string {
  return commands.map((command) => commandPromptText(command)).join('\n\n')
}

function commandPromptText(command: CommandSpec): string {
  const registry = new CommandRegistry()
  registry.register(command)
  return registry.renderPrompt()
}
