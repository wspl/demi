import type { AgentHarness } from '@demi/agent'
import { CommandRegistry, type CommandSpec, type Host } from '@demi/shell'
import { createEditorCommand } from './editor-command'
import { createFileReferenceResolver } from './reference-resolver'
import { createTodoCommand } from './todo-command'

export type CodingState = Record<string, never>

export interface CodingAgentHarnessOptions {
  host: Host
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

export function createCodingAgentHarness(options: CodingAgentHarnessOptions): AgentHarness<CodingState> {
  const commands = options.commands ?? defaultCodingCommands(options.host)
  const referenceHost = options.referenceHost ?? options.host
  const resolveReferences = createFileReferenceResolver<CodingState>(referenceHost)
  return {
    name: 'coding',
    initialState: () => ({}),
    host: () => options.host,
    commands: () => [...commands],
    systemPrompt: () => {
      const commandPrompt = renderCommandList(commands)
      const sections = [
        'You are a coding agent. Use shell session tools to inspect, edit, test, and verify the workspace.',
        'Treat cwd as the task workspace. Create, edit, and verify task files there by default; do not create a separate project directory under /tmp or another absolute path unless the user asks for it or the workspace is unusable.',
        'Prefer registered commands for agent-specific state and audited workflows. Use normal system commands for ordinary shell work.',
        [
          'Shell session rules:',
          '- Use shell_exec for commands. yieldAfterMs is required and only controls the initial observation window; it never kills the command.',
          '- Tool description: concise user-visible intent; no object-only labels, steps, tool names, ids, internal labels, or reasons.',
          '- shell_exec returns shellId, commandId, stdout/stderr deltas, and status. Track commandId for all follow-up control.',
          '- If status is running, call yield to end the current turn and schedule a wakeup, then call shell_status with commandId in the next turn.',
          '- Use shell_status for polling. It is non-blocking and reads new stdout/stderr since the last snapshot unless offsets are provided.',
          '- For dev servers, watch commands, previews, and other long-running processes that you need to observe and stop, run them in the foreground with a short yieldAfterMs, use yield + shell_status to observe, and shell_abort by commandId to stop. Avoid starting them with "&" and avoid pkill/killall by process name.',
          '- After a long-running process has been verified and stopped, summarize the observed evidence instead of restarting it to demonstrate the same behavior again.',
          '- If a foreground process is running and you need a separate one-off command, such as curl against a dev server, call shell_exec without shellId; keep using the original commandId to status, write to, or abort the long-running process.',
          '- Prefer non-interactive CLI flags for scaffolds and package tools when available.',
          '- For underspecified scaffold requests, choose a reasonable non-interactive default and proceed unless the choice is destructive or impossible.',
          '- Send non-empty stdin with shell_write only when the running command is known to be waiting for specific input; include a newline such as "Alice\\n" for line-oriented prompts.',
          '- For interactive stdin, keep the reader inside one foreground system process such as sh -c, node, or python; do not rely on the session script builtin read across turns.',
          '- Use shell_abort only when intentionally stopping a foreground command, and pass commandId.',
        ].join('\n'),
        'File references attached by the client are expanded before provider calls.',
      ]
      if (commandPrompt.trim()) sections.push(`Registered commands:\n\n${commandPrompt}`)
      return sections.join('\n\n')
    },
    resolveReferences,
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
