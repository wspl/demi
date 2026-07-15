import type { AgentHarness } from '@demicodes/agent'
import { CommandRegistry, type Command, type Host } from '@demicodes/shell'
import { createDemiCommand } from './demi-command'
import { createFileReferenceResolver } from './reference-resolver'
import { createTodoCommand } from './todo-command'

export type CodingState = Record<string, never>

export interface CodingAgentHarnessOptions {
  host: Host
  referenceHost?: Host
  commands?: Command[]
}

export interface CodingCommandRegistryOptions {
  includeDemi?: boolean
  commands?: Command[]
}

export function createCodingCommandRegistry(options: CodingCommandRegistryOptions = {}): CommandRegistry {
  const registry = new CommandRegistry()
  const commands = options.commands ?? [
    ...(options.includeDemi ? [createDemiCommand()] : []),
    createTodoCommand(),
  ]
  for (const command of commands) registry.register(command)
  return registry
}

export function createCodingAgentHarness(options: CodingAgentHarnessOptions): AgentHarness<CodingState> {
  const commands = options.commands ?? defaultCodingCommands()
  const referenceHost = options.referenceHost ?? options.host
  const resolveReferences = createFileReferenceResolver<CodingState>(referenceHost)
  return {
    name: 'coding',
    initialState: () => ({}),
    host: () => options.host,
    commands: () => [...commands],
    systemPrompt: (ctx) => {
      const sections = [
        'You are a coding agent. Use shell session tools to inspect, edit, test, and verify the workspace.',
        'Treat cwd as the task workspace. Create, edit, and verify task files there by default; do not create a separate project directory under /tmp or another absolute path unless the user asks for it or the workspace is unusable.',
        'Prefer registered commands for agent-specific state and audited workflows. Use normal system commands for ordinary shell work.',
        [
          'Shell session rules:',
          '- Use shell_exec for commands. timeoutMs is required and is only an observation window, not a kill deadline; at timeoutMs the command keeps running and a commandId is returned while the turn continues.',
          '- Tool description: concise title for the concrete user-visible state/result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.',
          '- shell_exec returns shellId, commandId, stdout/stderr deltas, and status. Track commandId for all follow-up control.',
          '- If status is running, the command keeps running and the turn continues. Either call shell_status again to check, or call yield to end this turn and be woken later to check with commandId. shell_exec and shell_status never end the turn or schedule a wakeup on their own; only yield does.',
          '- Use shell_status for polling. It is non-blocking and reads new stdout/stderr since the last snapshot unless offsets are provided.',
          '- For dev servers, watch commands, previews, and other long-running processes that you need to observe and stop, run them in the foreground with a short timeoutMs, use shell_status to observe (and yield to wait between checks), and shell_abort by commandId to stop. Avoid starting them with "&" and avoid pkill/killall by process name.',
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
      if (ctx.commandsPrompt.trim()) sections.push(`Registered commands:\n\n${ctx.commandsPrompt}`)
      return sections.join('\n\n')
    },
    resolveReferences,
  }
}

function defaultCodingCommands(): Command[] {
  return [createDemiCommand(), createTodoCommand()]
}
