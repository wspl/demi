// Runs a package script's commands one after another in the foreground, for
// the scripts that would otherwise chain them with `&&`. `bun run` passes a
// stop or an interrupt on to the command it runs only when the script line is
// a single command: a line with `&&` runs in a shell, which dies on SIGTERM
// and leaves the command it started running. So such a script is one Bun
// program that runs its steps through `runSteps`, and stopping the program's
// process id stops the step that is running.
import { constants } from 'node:os'

/** One command of a script, and the directory it runs in. */
export interface Step {
  command: string[]
  cwd: string
  /**
   * Runs the step in a session of its own, out of reach of a signal sent to
   * this program's process group, such as the SIGKILL that stops a shell job
   * or a terminal's Ctrl-C; this program passes SIGINT, SIGTERM and SIGHUP on
   * to it. For a step that stops what it started in order and so must not be
   * killed with the group, and that ends by itself when this program ends,
   * which `xtask` does.
   */
  detached?: boolean
}

/** The signals that stop the running step and every later one. */
const stops = ['SIGINT', 'SIGTERM', 'SIGHUP'] as const

/**
 * Runs `steps` one after another with this process's terminal and
 * environment, and ends this process with the first failing step's exit code,
 * or the last step's. SIGINT, SIGTERM and SIGHUP reach the running step,
 * which stops what it started; no step starts after one of them.
 */
export async function runSteps(steps: Step[]): Promise<never> {
  let child: Bun.Subprocess | undefined
  let stop: (typeof stops)[number] | undefined
  for (const signal of stops) {
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
      env: process.env,
      stdio: ['inherit', 'inherit', 'inherit'],
      detached: step.detached ?? false,
    })
    code = await child.exited
    if (code !== 0) {
      break
    }
  }
  process.exit(code)
}
