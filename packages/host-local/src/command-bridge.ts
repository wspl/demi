import { existsSync, mkdirSync, unlinkSync } from 'node:fs'
import { dirname } from 'node:path'
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import type { Server } from 'node:net'
import {
  RunCommandLineCommandNotRegisteredError,
  RunCommandLineSessionNotFoundError,
  RunCommandLineTimeoutError,
  type AgentServer,
} from '@demicodes/agent'
import { errorMessage } from '@demicodes/utils'

/**
 * Dispatch script shared by every generated command-name symlink.
 * Reads the invoked name from the symlink path, applies the stdin grace
 * contract from docs/command-bridge.md, and POSTs /run on the UDS socket.
 */
export const COMMAND_BRIDGE_SHIM_SOURCE = `#!/usr/bin/env node
const { request } = require('node:http')
const { basename } = require('node:path')

// Stdin policy (docs/command-bridge.md): if no byte arrives within the grace
// window, proceed with empty stdin. Once any byte arrives, clear the timer
// and read until EOF with no time cap. Late data after empty-stdin dispatch
// is reported on stderr and not pretended to have been delivered.
const STDIN_GRACE_MS = 300

function readStdin() {
  if (process.stdin.isTTY) return Promise.resolve('')
  return new Promise((resolve, reject) => {
    const chunks = []
    let resolved = false
    let timedOut = false
    let timer
    const cleanup = () => {
      clearTimeout(timer)
      process.stdin.removeListener('data', onData)
      process.stdin.removeListener('end', onEnd)
      process.stdin.removeListener('error', onError)
    }
    const onData = (chunk) => {
      if (timedOut) {
        process.stderr.write(
          'command bridge: ' + chunk.length + ' byte(s) of stdin arrived after the ' + STDIN_GRACE_MS +
            'ms grace period elapsed; the command already ran with empty stdin\\n',
        )
        cleanup()
        return
      }
      clearTimeout(timer)
      chunks.push(chunk)
    }
    const onEnd = () => {
      if (timedOut) {
        cleanup()
        return
      }
      resolved = true
      cleanup()
      resolve(Buffer.concat(chunks).toString('utf8'))
    }
    const onError = (error) => {
      if (resolved || timedOut) return
      resolved = true
      cleanup()
      reject(error)
    }
    timer = setTimeout(() => {
      timedOut = true
      resolved = true
      resolve('')
    }, STDIN_GRACE_MS)
    process.stdin.on('data', onData)
    process.stdin.on('end', onEnd)
    process.stdin.on('error', onError)
  })
}

function postRun(socketPath, body) {
  return new Promise((resolve, reject) => {
    const req = request(
      { socketPath, path: '/run', method: 'POST', headers: { 'content-type': 'application/json' } },
      (res) => {
        const chunks = []
        res.on('data', (chunk) => chunks.push(chunk))
        res.on('end', () => resolve({ statusCode: res.statusCode || 0, body: Buffer.concat(chunks).toString('utf8') }))
      },
    )
    req.on('error', reject)
    req.end(body)
  })
}

async function main() {
  const socketPath = process.env.DEMI_COMMAND_BRIDGE_SOCK
  // Shell sessions export DEMI_SESSION_ID as the command scope id (the agent
  // session id for agent-owned shells).
  const commandScopeId = process.env.DEMI_SESSION_ID
  if (!socketPath || !commandScopeId) {
    process.stderr.write('command bridge: DEMI_COMMAND_BRIDGE_SOCK / session id not set in this shell\\n')
    process.exit(1)
  }
  const name = basename(process.argv[1] || '')
  const args = process.argv.slice(2)
  const stdin = await readStdin()
  const body = JSON.stringify({ commandScopeId, name, args, cwd: process.cwd(), stdin })

  const response = await postRun(socketPath, body)
  if (response.statusCode !== 200) {
    let message = response.body
    try {
      message = JSON.parse(response.body).error || message
    } catch {}
    process.stderr.write('command bridge: ' + message + '\\n')
    process.exit(1)
  }
  const result = JSON.parse(response.body)
  if (result.stdout) process.stdout.write(result.stdout)
  if (result.stderr) process.stderr.write(result.stderr)
  process.exit(result.exitCode)
}

main().catch((error) => {
  process.stderr.write('command bridge: ' + (error && error.message ? error.message : String(error)) + '\\n')
  process.exit(1)
})
`

export interface CommandBridgeOptions {
  socketPath: string
}

export interface CommandBridgeHandle {
  close(): Promise<void>
}

interface RunRequestBody {
  commandScopeId: string
  name: string
  args: string[]
  cwd: string
  stdin: string
}

/**
 * Starts the process-wide UDS listener for command-bridge shims.
 * Each POST /run is dispatched to `AgentServer.runCommandLine`.
 */
export function startCommandBridge(server: AgentServer, options: CommandBridgeOptions): CommandBridgeHandle {
  mkdirSync(dirname(options.socketPath), { recursive: true })
  if (existsSync(options.socketPath)) unlinkSync(options.socketPath)

  const httpServer = createServer((req, res) => {
    void handleRun(server, req, res)
  })
  httpServer.listen(options.socketPath)

  return {
    close: () => closeServer(httpServer, options.socketPath),
  }
}

async function handleRun(server: AgentServer, req: IncomingMessage, res: ServerResponse): Promise<void> {
  if (req.method !== 'POST' || req.url !== '/run') {
    sendJson(res, 404, { error: 'not found' })
    return
  }

  let body: RunRequestBody
  try {
    body = await readJsonBody(req)
  } catch (error) {
    sendJson(res, 400, { error: `bad request: ${errorMessage(error)}` })
    return
  }
  if (!isRunRequestBody(body)) {
    sendJson(res, 400, { error: 'bad request: expected { commandScopeId, name, args, cwd, stdin }' })
    return
  }

  const controller = new AbortController()
  let responded = false
  const onClosedEarly = (): void => {
    if (!responded) controller.abort()
  }
  req.on('close', onClosedEarly)

  try {
    const result = await server.runCommandLine(body.commandScopeId, body.name, body.args, {
      cwd: body.cwd,
      stdin: body.stdin,
      signal: controller.signal,
    })
    responded = true
    sendJson(res, 200, result)
  } catch (error) {
    responded = true
    sendJson(res, statusForError(error), { error: errorMessage(error) })
  } finally {
    req.off('close', onClosedEarly)
  }
}

function statusForError(error: unknown): number {
  if (error instanceof RunCommandLineSessionNotFoundError) return 404
  if (error instanceof RunCommandLineCommandNotRegisteredError) return 404
  if (error instanceof RunCommandLineTimeoutError) return 504
  return 500
}

function isRunRequestBody(value: unknown): value is RunRequestBody {
  if (value === null || typeof value !== 'object') return false
  const record = value as Record<string, unknown>
  return (
    typeof record.commandScopeId === 'string' &&
    typeof record.name === 'string' &&
    Array.isArray(record.args) &&
    record.args.every((arg) => typeof arg === 'string') &&
    typeof record.cwd === 'string' &&
    typeof record.stdin === 'string'
  )
}

function readJsonBody(req: IncomingMessage): Promise<RunRequestBody> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    req.on('data', (chunk: Buffer) => chunks.push(chunk))
    req.on('end', () => {
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString('utf8')) as RunRequestBody)
      } catch (error) {
        reject(error)
      }
    })
    req.on('error', reject)
  })
}

function sendJson(res: ServerResponse, status: number, body: unknown): void {
  const text = JSON.stringify(body)
  res.writeHead(status, { 'content-type': 'application/json; charset=utf-8' })
  res.end(text)
}

function closeServer(httpServer: Server, socketPath: string): Promise<void> {
  return new Promise((resolve) => {
    httpServer.close(() => {
      try {
        if (existsSync(socketPath)) unlinkSync(socketPath)
      } catch {
        // best-effort cleanup of the filesystem socket
      }
      resolve()
    })
  })
}
