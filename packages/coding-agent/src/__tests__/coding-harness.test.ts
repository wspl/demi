import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import {
  AgentSession,
  createStandardAgentTools,
  injectSubagentCommand,
  subagentCommandShape,
  type AgentHarness,
  type AgentHarnessRuntime,
} from '@demicodes/agent'
import { StubProvider, events } from '@demicodes/provider/testing'
import {
  CommandRegistry,
  type Command,
  type ShellEnvironment
} from '@demicodes/shell'
import { runnerShell, runnerShellFactory } from '@demicodes/backend/testing'
import { LocalHost } from '@demicodes/runner/testing'
import { createCodingAgentHarness } from '../index'

const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 100_000,
    outputLimit: null,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

test(
  'coding agent harness exposes shell session tools and registered command prompt',
  async () => {
    const harness = createCodingAgentHarness({
      host: new LocalHost(process.cwd())
    })
    const state = harness.initialState()
    const commands = (await harness.commands?.({
      state,
      cwd: process.cwd(),
      agentSessionId: 'test-session'
    })) ?? []
    const { environment, runtime } = await createRuntimeFromHarness(
      harness,
      process.cwd()
    )

    expect(harness.name).toBe('coding')
    expect(commands.map((command) => command.name)).toEqual(['demi'])
    const tools = runtime.tools({
      agentSessionId: 'coding-test-agent',
      state,
      cwd: process.cwd(),
      metadata: null
    })
    expect(tools.map((tool) => tool.name)).toEqual([
      'shell_exec',
      'shell_status',
      'shell_write',
      'shell_abort',
      'yield',
    ])
    for (const tool of tools) {
      const properties = tool.inputSchema.properties as Record<string, unknown>
        | undefined
      expect(properties?.description).toEqual(expect.objectContaining({
        type: 'string'
      }))
    }
    const prompt = await harness.systemPrompt({
      agentSessionId: 'coding-test-agent',
      state,
      cwd: process.cwd(),
      transcript: {} as never,
      commandsPrompt: renderCommandsPrompt(commands),
      metadata: null,
    })
    expect(prompt).toContain('demi: The Demi platform command')
    expect(prompt).toContain('Treat cwd as the task workspace')
    expect(prompt)
      .toContain('do not create a separate project directory under /tmp')
    expect(prompt).toContain('demi file create')
    expect(prompt)
      .toContain('Success output: writes "Created <path>" to stdout')
    expect(prompt).toContain(
      'Failure output: writes the reason to stderr and exits non-zero'
    )
    expect(prompt).toContain('todo: Manage an agent-session-scoped task list')
    expect(prompt)
      .toContain('run them in the foreground with a short timeoutMs')
    expect(prompt).toContain(
      'Tool description: concise title for the concrete user-visible state/result'
    )
    expect(prompt).toContain('Do not describe waiting, pausing, tool mechanics')
    expect(prompt).toContain(
      'shell_status to observe (and yield to wait between checks)'
    )
    expect(prompt).toContain('avoid pkill/killall by process name')
    expect(prompt).toContain(
      'instead of restarting it to demonstrate the same behavior again'
    )
    expect(prompt).toContain(
      'include a newline such as "Alice\\n" for line-oriented prompts'
    )
    expect(prompt).toContain(
      'do not rely on the session script builtin read across turns'
    )
    expect(prompt).toContain(
      'File references attached by the client are expanded before provider calls.'
    )

    const todo = await environment.exec({
      script: 'demi todo add "Verify default registration"'
    })
    expect(todo.stdout.delta).toBe('[ ] T1 Verify default registration\n')
    const demiHelp = await environment.exec({
      shellId: todo.shellId,
      script: 'demi --help'
    })
    expect(demiHelp.stdout.delta).toContain('demi file create')
    expect(demiHelp.stdout.delta)
      .toContain('Success output: writes "Created <path>" to stdout')
  }
)

test(
  'coding agent harness ships no named profiles; omitting --profile inherits',
  async () => {
    const harness = createCodingAgentHarness({
      host: new LocalHost(process.cwd())
    })
    const state = harness.initialState()
    const profiles = (await harness.agents?.({ state, cwd: process.cwd() }))
      ?? []

    expect(profiles).toEqual([])
  }
)

test(
  'the injected demi agent command help teaches self-contained spawn prompts',
  async () => {
    const harness = createCodingAgentHarness({
      host: new LocalHost(process.cwd())
    })
    const state = harness.initialState()
    const harnessContext = {
      state,
      cwd: process.cwd(),
      agentSessionId: 'test-session'
    }
    const commands = (await harness.commands?.(harnessContext)) ?? []
    const profiles = (await harness.agents?.(harnessContext)) ?? []
    const agentCommands = subagentCommandShape(profiles.map((profile) => profile.name))
    const registry = new CommandRegistry()
    for (const command of injectSubagentCommand(
      commands,
      agentCommands
    )) registry.register(command)
    const help = registry.renderHelp()

    // Spawn is grafted under the harness's existing demi root, beside file editing.
    expect(help).toContain('demi file create')
    expect(help).toContain('demi agent')
    expect(help).toContain('demi agent steer')
    expect(help).toContain('demi agent abort')
    expect(help).toContain('demi agent list')
    expect(help).toContain('demi agent show')
    for (const field of ['content', 'patch', 'prompt', 'message']) {
      expect(help).toContain(`Stdin body: ${field}`)
      expect(help).not.toContain(`--${field}`)
    }
    expect(help).toContain("demi agent send <id> [--json] <<'EOF'")
    expect(help).toContain('--status <pending|in_progress|done>')
    // The prompt field teaches that the child cannot see this conversation.
    expect(help).toContain('cannot see this conversation')
    expect(help).toContain(
      'State the exact shape of the last assistant text it should return.'
    )
    expect(help).toContain('Available: none')
  }
)

test('coding agent harness leaves shell lifecycle to host assembly', () => {
  const harness = createCodingAgentHarness({
    host: new LocalHost(process.cwd())
  })

  expect('tools' in harness).toBe(false)
  expect(harness.dispose).toBeUndefined()
})

async function createRuntimeFromHarness(
  harness: AgentHarness<Record<string, never>>,
  cwd: string,
): Promise<{
  environment: ShellEnvironment;
  runtime: AgentHarnessRuntime<Record<string, never>>;
  state: Record<string, never>
}> {
  const state = harness.initialState()
  const harnessContext = { state, cwd, agentSessionId: 'test-session' }
  const registry = new CommandRegistry()
  const commands = harness.commands?.(harnessContext) ?? []
  if (commands instanceof Promise)
    throw new Error('test harness commands must be synchronous')
  for (const command of commands) registry.register(command)
  const host = harness.host(harnessContext)
  if (host instanceof Promise)
    throw new Error('test harness host must be synchronous')
  const environment = await runnerShell({
    host,
    commands: registry,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })
  const runtime: AgentHarnessRuntime<Record<string, never>> = {
    harnessName: harness.name,
    initialState: () => state,
    systemPrompt: (ctx) => harness.systemPrompt({
      ...ctx,
      commandsPrompt: registry.renderHelp()
    }),
    preamble: (ctx) => harness.preamble?.(ctx) ?? null,
    lifecycle: (event) => harness.lifecycle?.(event),
    tools: () =>
      createStandardAgentTools({
        environment,
        scheduleYield: (_ctx, durationMs) => ({
          output: [{
            type: 'text',
            text: `yield scheduled\nwakeupId: test\ndurationMs: ${durationMs}`
          }],
          stopAfterToolResult: true,
        }),
      }),
  }
  return { environment, runtime, state }
}

function renderCommandsPrompt(commands: readonly Command[]): string {
  const registry = new CommandRegistry()
  for (const command of commands) registry.register(command)
  return registry.renderHelp()
}
