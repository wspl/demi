import { defineCommand, type Command, type CommandGroup, type CommandIO } from '@demicodes/shell'
import { z } from 'zod'
import { reachableHosts } from '../conversation/hosts'
import type { HostCommandDeps } from '../runner/host-command'
import { ExposeError, EXPOSE_LIFETIME_MS, type Exposes } from './records'
import type { ControlService, ExposeRecord } from '../storage/control'

const jsonSchema = z.object({
  id: z.string(),
  deviceId: z.string(),
  address: z.string(),
  url: z.string(),
  createdAt: z.string(),
  expiresAt: z.string()
})

/**
 * The `demi host expose` subcommand group (`expose.md` § Commands): its
 * leaves are `rpc`, the backend owns the records. `add` names the device the
 * conversation names — its main host, or an attached one through `--host`.
 */
export function createExposeCommandGroup(
  deps: HostCommandDeps,
  conversationId: string
): CommandGroup {
  return {
    name: 'expose',
    summary: 'Give a service on a host a public URL for one hour: add, list, renew, remove.',
    subcommands: [
      addCommand(deps, conversationId),
      listCommand(deps, conversationId),
      renewCommand(deps, conversationId),
      removeCommand(deps, conversationId),
    ],
  }
}

function addCommand(
  deps: HostCommandDeps,
  conversationId: string
): Command {
  return defineCommand({
    name: 'add',
    summary: 'Expose a service on a host under a fresh public URL for one hour: `demi host expose add <host:port|port> [--host <name|id>]`.',
    successOutput: 'the URL, the device, and the expiry, or JSON matching { expose } when --json is passed',
    failureOutput: 'expose_unavailable without an expose domain, an unreachable --host, or a device that is not connected; writes the reason to stderr and exits non-zero',
    input: {
      address: z.string().min(1).describe('host:port, or a bare port meaning 127.0.0.1'),
      host: z.string().optional().describe('Host name or device id from demi host list; the main host by default'),
    },
    positionals: ['address'],
    output: { json: z.object({ expose: jsonSchema }) },
    kind: 'rpc',
    run: async ({ parsed, io }) => {
      const hosts = await reachableHosts(deps, conversationId)
      const wanted = parsed.values.host
      const target = wanted === undefined
        ? hosts.find(host => host.role === 'main')
        : hosts.find(host => host.name === wanted || host.deviceId === wanted)
      if (!target) {
        await io.stderr(
          `expose add: host ${wanted ?? '(main)'} is not reachable from this conversation (see \`demi host list\`)\n`
        )
        return { exitCode: 1 }
      }
      const device = await deps.control.getDevice(target.deviceId)
      let record: ExposeRecord
      try {
        record = await deps.exposes.add(
          await conversationUser(deps.control, conversationId),
          device!,
          parsed.values.address
        )
      } catch (error) {
        if (error instanceof ExposeError) {
          await io.stderr(`expose add: ${error.message} (${error.code})\n`)
          return { exitCode: 1 }
        }
        throw error
      }
      await io.stdout(parsed.json
        ? JSON.stringify({ expose: deps.exposes.view(record) })
        : `Exposed ${record.address} on ${device!.name} as ${deps.exposes.url(record)}\n` +
          `Expires in ${EXPOSE_LIFETIME_MS / 60_000} minutes (expose ${record.id}).\n`)
      return { exitCode: 0 }
    },
  })
}

function listCommand(deps: HostCommandDeps, conversationId: string): Command {
  return defineCommand({
    name: 'list',
    summary: 'Every expose of this user across devices, soonest expiry first.',
    successOutput: 'one line per expose, or JSON matching { exposes } when --json is passed',
    failureOutput: 'never fails on live data',
    output: { json: z.object({ exposes: z.array(jsonSchema) }) },
    kind: 'rpc',
    run: async ({ parsed, io }) => {
      const records = await deps.exposes.list(
        await conversationUser(deps.control, conversationId)
      )
      if (parsed.json) {
        await io.stdout(JSON.stringify({
          exposes: records.map(record => deps.exposes.view(record))
        }))
        return { exitCode: 0 }
      }
      if (records.length === 0) {
        await io.stdout('No exposes.\n')
        return { exitCode: 0 }
      }
      const names = new Map(
        (await Promise.all([...new Set(records.map(record => record.deviceId))]
          .map(async id => [id, (await deps.control.getDevice(id))?.name ?? id] as const)))
      )
      const lines = records.map(record => {
        const minutes = Math.max(0, Math.round(
          (Date.parse(record.expiresAt) - Date.now()) / 60_000
        ))
        return `${record.id}  ${names.get(record.deviceId)}  ${record.address}  ${minutes} min  ${deps.exposes.url(record)}`
      })
      await io.stdout(`${lines.join('\n')}\n`)
      return { exitCode: 0 }
    },
  })
}

function renewCommand(deps: HostCommandDeps, conversationId: string): Command {
  return defineCommand({
    name: 'renew',
    summary: "Set an expose's expiry to one hour from now.",
    successOutput: 'the new expiry, or JSON matching { expose } when --json is passed',
    failureOutput: 'expose_not_found when the id is not this user\'s or has expired; exits non-zero',
    input: { id: z.string().min(1).describe('Expose id') },
    positionals: ['id'],
    output: { json: z.object({ expose: jsonSchema }) },
    kind: 'rpc',
    run: async ({ parsed, io }) => {
      try {
        const record = await deps.exposes.renew(
          await conversationUser(deps.control, conversationId),
          parsed.values.id
        )
        await io.stdout(parsed.json
          ? JSON.stringify({ expose: deps.exposes.view(record) })
          : `Expose ${record.id} expires in ${EXPOSE_LIFETIME_MS / 60_000} minutes.\n`)
        return { exitCode: 0 }
      } catch (error) {
        return notFoundOutput(error, 'renew', io)
      }
    },
  })
}

function removeCommand(deps: HostCommandDeps, conversationId: string): Command {
  return defineCommand({
    name: 'remove',
    summary: 'Destroy an expose at once; its URL no longer works.',
    successOutput: 'confirms the removal',
    failureOutput: 'expose_not_found when the id is not this user\'s or has expired; exits non-zero',
    input: { id: z.string().min(1).describe('Expose id') },
    positionals: ['id'],
    kind: 'rpc',
    run: async ({ parsed, io }) => {
      const userId = await conversationUser(deps.control, conversationId)
      try {
        await deps.exposes.remove(userId, parsed.values.id)
      } catch (error) {
        return notFoundOutput(error, 'remove', io)
      }
      await io.stdout(
        `Removed expose ${parsed.values.id}; its URL no longer works.\n`
      )
      return { exitCode: 0 }
    },
  })
}

/** An id that is not the user's, or that has expired, answers `expose_not_found` on every leaf. */
async function notFoundOutput(error: unknown, leaf: string, io: CommandIO): Promise<{ exitCode: number }> {
  if (error instanceof ExposeError) {
    await io.stderr(`expose ${leaf}: ${error.message} (${error.code})\n`)
    return { exitCode: 1 }
  }
  throw error
}

/** The conversation's owner: exposes belong to the user, not the conversation. */
async function conversationUser(
  control: ControlService,
  conversationId: string
): Promise<string> {
  const conversation = await control.getConversation(conversationId)
  if (!conversation)
    throw new Error('expose: this session has no conversation record')
  return conversation.userId
}
