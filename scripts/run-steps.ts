// Runs a package script's commands one after another in the foreground, for
// the scripts that would otherwise chain them with `&&`. `bun run` passes a
// stop or an interrupt on to the command it runs only when the script line is
// a single command: a line with `&&` runs in a shell, which dies on SIGTERM
// and leaves the command it started running. So such a script is one Bun
// program that runs its steps through `runSteps`, and stopping the program's
// process id stops the step that is running.
import { constants } from 'node:os'

/**
 * One command of a script, the directory it runs in, and its environment
 * when not this process's.
 */
export interface Step {
  command: string[]
  cwd: string
  env?: Record<string, string | undefined>
}

/**
 * Runs `steps` one after another with this process's terminal and
 * environment, and ends this process with the first failing step's exit code,
 * or the last step's. SIGINT and SIGTERM reach the running step, which stops
 * what it started; no step starts after one of them.
 */
export async function runSteps(steps: Step[]): Promise<never> {
  let child: Bun.Subprocess | undefined
  let stop: 'SIGINT' | 'SIGTERM' | undefined
  for (const signal of ['SIGINT', 'SIGTERM'] as const) {
    process.on(signal, () => {
      stop = signal
      // Between steps the last child has exited, and killing it does nothing.
      child?.kill(signal)
    })
  }
  let code = 0
  for (const step of steps) {
    if (stop) {
      process.exit(128 + constants.signals[stop])
    }
    // Bun reads the repository's .env into this process's environment but
    // not into a package script's, and a child gets the environment this
    // process started with unless given `process.env`: so the .env's
    // variables (DEMI_DEV_PROVIDER_* for `xtask dev`) reach the steps.
    child = Bun.spawn(step.command, {
      cwd: step.cwd,
      env: step.env ?? process.env,
      stdio: ['inherit', 'inherit', 'inherit'],
    })
    code = await child.exited
    if (code !== 0) {
      break
    }
  }
  process.exit(code)
}
