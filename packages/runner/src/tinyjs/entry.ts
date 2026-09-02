// The entry of the tinyjs bundle (`docs/demi-next/tinyjs.md` § Entry modes):
// one packed binary reached through symlinks, the mode chosen by the name it
// was invoked by. `demi-runner` is runner mode; any other name is a root
// command in command mode.
import { argv, exit, stderrWriter } from '@demicodes/host-runner'
import { basenamePath } from '@demicodes/utils'
import { runCommandMode } from './command-mode'

const RUNNER_NAME = 'demi-runner'

async function main(): Promise<number> {
  const name = basenamePath(argv[0] ?? '')
  if (name === RUNNER_NAME) {
    await stderrWriter()(`${RUNNER_NAME}: runner mode is not on tinyjs yet (M9, the runner port)\n`)
    return 2
  }
  return runCommandMode(name, argv.slice(1))
}

exit(await main())
