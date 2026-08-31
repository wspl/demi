import process from 'node:process'
import { RunnerClient } from './runner-client'
import { RunnerState } from './state'

function usage(): never {
  console.error('Usage: demi-runner run [--backend <url>]')
  process.exit(2)
}

async function main(): Promise<void> {
  const args = process.argv.slice(2)
  if (args[0] !== 'run') usage()

  let backendUrl: string | null = null
  for (let index = 1; index < args.length; index += 1) {
    if (args[index] === '--backend') {
      backendUrl = args[index + 1] ?? null
      index += 1
    } else {
      usage()
    }
  }

  const state = new RunnerState()
  backendUrl ??= (await state.readConfig())?.backendUrl ?? null
  if (!backendUrl) {
    console.error('No backend URL: pass --backend <url> on first start.')
    process.exit(2)
  }

  const client = new RunnerClient({
    backendUrl,
    onStatus: (status, detail) => {
      if (status === 'online') console.log('runner online')
      if (status === 'connecting') console.log('connecting…')
      if (status === 'rejected') console.error(`rejected by backend: ${detail ?? 'unknown reason'}`)
    },
    onClaimPending: (claimToken) => {
      console.log(`Unclaimed device. Enter this claim token in the web UI: ${claimToken}`)
    },
  })
  client.start()

  const shutdown = () => {
    void client.stop().then(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

void main()
