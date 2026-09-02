import type { DispatchIO, HostFileSystem, RootPaths } from '@demicodes/shell'
import type { Command, List, Pipeline, Script } from '../grammar/ast'
import { type ExpansionScope, expandArgv, expandSingle } from '../grammar/expand'
import { BUILTINS } from '../builtins/table'
import type { BuiltinContext } from '../builtins/io'
import { type Channels, RedirectError, applyRedirects } from './redirect'
import { emptyByteStream, isAbortError } from '@demicodes/utils'
import { Pipe, PipeClosed, type Writer } from './stream'
import { OutsideError } from '../outside/reasons'

export interface ShellState {
  cwd: string
  home: string
  vars: Record<string, string>
}

export interface ExecutionEnvironment {
  fs: HostFileSystem
  state: ShellState
  roots: ReadonlyMap<string, RootPaths>
  dispatch: (root: string, argv: string[], io: DispatchIO) => Promise<number>
  identity: { user: string; group: string }
  stdout: Writer
  stderr: Writer
  /** The script's stdin, shared by the root commands whose stdin is not redirected. */
  stdin?: AsyncIterable<Uint8Array>
  signal?: AbortSignal
}

/** Runs a checked script: statements in order, chains by exit status, pipelines concurrently. */
export async function executeScript(script: Script, env: ExecutionEnvironment): Promise<number> {
  let last = 0
  for (const statement of script.statements) {
    if (env.signal?.aborted) return 130
    last = await runList(statement, env)
  }
  return last
}

async function runList(list: List, env: ExecutionEnvironment): Promise<number> {
  let status = await runPipeline(list.first, env)
  for (const { op, pipeline } of list.rest) {
    if (env.signal?.aborted) return 130
    if (op === '&&' ? status !== 0 : status === 0) continue
    status = await runPipeline(pipeline, env)
  }
  return status
}

async function runPipeline(pipeline: Pipeline, env: ExecutionEnvironment): Promise<number> {
  const top: Channels = { stdin: emptyByteStream(), stdout: env.stdout, stderr: env.stderr }
  if (pipeline.commands.length === 1) {
    return runCommand(pipeline.commands[0]!, top, env, true, env.stdin)
  }
  // Every command of a pipeline runs in a subshell: no cd or assignment escapes.
  const pipes = pipeline.commands.slice(0, -1).map(() => new Pipe())
  const runs = pipeline.commands.map(async (command, index) => {
    const stdin = index === 0 ? top.stdin : pipes[index - 1]!
    const stdout: Writer = index === pipes.length ? top.stdout : pipes[index]!.write
    let status: number
    try {
      status = await runCommand(command, { stdin, stdout, stderr: top.stderr }, env, false, index === 0 ? env.stdin : undefined)
    } finally {
      if (index < pipes.length) pipes[index]!.close()
      if (index > 0) pipes[index - 1]!.abandon()
    }
    return status
  })
  const statuses = await Promise.all(runs)
  return statuses[statuses.length - 1]!
}

async function runCommand(command: Command, inherited: Channels, env: ExecutionEnvironment, canMutate: boolean, scriptStdin?: AsyncIterable<Uint8Array>): Promise<number> {
  const state = env.state
  const scope: ExpansionScope = { home: state.home, cwd: state.cwd, vars: state.vars }
  if (command.words.length === 0) {
    if (canMutate) for (const assignment of command.assignments) state.vars[assignment.name] = expandSingle(assignment.value, scope)
    return 0
  }
  const argv = await expandArgv(command.words, scope, env.fs)
  let redirected: Awaited<ReturnType<typeof applyRedirects>>
  try {
    redirected = await applyRedirects(command.redirects, inherited, scope, env.fs)
  } catch (error) {
    if (error instanceof RedirectError) {
      await inherited.stderr(`bash: line ${command.line}: ${error.path}: ${error.detail}\n`)
      return 1
    }
    throw error
  }
  if (argv.length === 0) {
    await redirected.flush()
    return 0
  }
  const commandEnv: Record<string, string> = { ...state.vars, PWD: state.cwd }
  for (const assignment of command.assignments) commandEnv[assignment.name] = expandSingle(assignment.value, scope)
  const { channels } = redirected
  const name = argv[0]!
  let status: number
  try {
    const builtin = BUILTINS.get(name)
    if (builtin) {
      const ctx: BuiltinContext = {
        argv: argv.slice(1),
        stdin: channels.stdin,
        stdout: channels.stdout,
        stderr: channels.stderr,
        fs: env.fs,
        cwd: state.cwd,
        home: state.home,
        env: commandEnv,
        identity: env.identity,
        signal: env.signal,
        line: command.line,
        shell: {
          setCwd: (path) => {
            if (canMutate) state.cwd = path
          },
        },
      }
      status = await builtin.run(ctx)
    } else {
      // A root command whose stdin is the script's own gets the live stream too.
      const stdinStream = channels.stdin === inherited.stdin ? scriptStdin : undefined
      status = await env.dispatch(name, argv.slice(1), {
        stdin: channels.stdin,
        stdinStream,
        stdout: channels.stdout,
        stderr: channels.stderr,
        cwd: state.cwd,
        env: commandEnv,
        signal: env.signal,
      })
    }
  } catch (error) {
    if (error instanceof PipeClosed) {
      status = 141
    } else if (isAbortError(error)) {
      status = 130
    } else if (error instanceof OutsideError) {
      // The parse-first check saw the words as written; a glob can expand into a word (a file
      // named like an option) that a builtin's whitelist does not cover. Reported, not crashed.
      await channels.stderr(`${error.message}\n`)
      status = 2
    } else {
      throw error
    }
  }
  await redirected.flush()
  return status
}
