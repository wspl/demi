import process from 'node:process'
import { errorMessage } from '@demicodes/utils'
import type { UserContentBlock } from '@demicodes/core'
import type { AbortResult } from '@demicodes/agent'
import { writeEventLine, writeLineTo, type ReplOutput } from './output'
import { finishStream, type RenderState } from './render'

interface ReplCommandClient {
  abort(): Promise<AbortResult>
  steer(content: UserContentBlock[]): Promise<void>
  retry(): Promise<void>
  resume(): Promise<void>
  compact(): Promise<void>
  shellWrite(commandId: string, stdin: string): Promise<void>
}

interface ReplLoopClient extends ReplCommandClient {
  send(content: UserContentBlock[]): Promise<void>
}

export interface ReplInputLoop {
  ask(): Promise<string>
  client: ReplLoopClient
  renderer: RenderState
  output?: ReplOutput
}

export const helpText = `Commands:
  /help                      Show this help
  /abort                     Abort the active turn
  /steer <message>           Steer the active turn without queueing a new turn
  /retry                     Retry the latest user turn
  /resume                    Resume after an abort
  /compact                   Request transcript compaction
  /input <commandId> <text>  Send stdin to a running command
  /exit                      Close the session

Tips:
  Start in a scratch directory for acceptance tests.
  Messages are sent asynchronously, so /abort can be typed while a turn is running.
  Example prompt: "Create src/app.ts, add a todo to run tests, then run cat src/app.ts."`

export async function runInputLoop(options: ReplInputLoop & { shouldContinue?: () => boolean }): Promise<void> {
  const output = options.output ?? process.stdout
  while (options.shouldContinue?.() ?? true) {
    const input = (await options.ask()).trim()
    if (!input) continue
    if (input.startsWith('/')) {
      const shouldExit = await handleCommand(input, options.client, output)
      if (shouldExit) break
      continue
    }

    const content: UserContentBlock[] = [{ type: 'text', text: input }]
    void options.client.send(content).catch((error) => {
      finishStream(options.renderer)
      writeEventLine(output, 'error', `send failed: ${errorMessage(error)}`, 'red')
    })
  }
}

export async function handleCommand(
  input: string,
  client: ReplCommandClient,
  output: ReplOutput = process.stdout,
): Promise<boolean> {
  const [command, ...rest] = input.split(/\s+/)
  switch (command) {
    case '/help':
      writeLineTo(output, helpText)
      return false
    case '/abort':
      writeEventLine(output, 'state', 'abort requested', 'yellow')
      void client.abort().catch((error) => writeEventLine(output, 'error', `abort failed: ${errorMessage(error)}`, 'red'))
      return false
    case '/steer': {
      const message = rest.join(' ').trim()
      if (!message) {
        writeEventLine(output, 'error', 'usage: /steer <message>', 'red')
        return false
      }
      void client
        .steer([{ type: 'text', text: message }])
        .catch((error) => writeEventLine(output, 'error', `steer failed: ${errorMessage(error)}`, 'red'))
      return false
    }
    case '/retry':
      void client.retry().catch((error) => writeEventLine(output, 'error', `retry failed: ${errorMessage(error)}`, 'red'))
      return false
    case '/resume':
      void client.resume().catch((error) => writeEventLine(output, 'error', `resume failed: ${errorMessage(error)}`, 'red'))
      return false
    case '/compact':
      void client.compact().catch((error) => writeEventLine(output, 'error', `compact failed: ${errorMessage(error)}`, 'red'))
      return false
    case '/input': {
      const commandId = rest.shift()
      if (!commandId) {
        writeEventLine(output, 'error', 'usage: /input <commandId> <text>', 'red')
        return false
      }
      void client
        .shellWrite(commandId, `${rest.join(' ')}\n`)
        .catch((error) => writeEventLine(output, 'error', `input failed: ${errorMessage(error)}`, 'red'))
      return false
    }
    case '/exit':
    case '/quit':
      return true
    default:
      writeEventLine(output, 'error', `unknown command: ${command}`, 'red')
      return false
  }
}
