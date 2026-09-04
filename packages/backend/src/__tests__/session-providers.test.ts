import { expect, test } from 'bun:test'
import { defineProvider, providerRuntime, type AgentProvider, type InferenceRequest, type ProviderSelection } from '@demicodes/provider'
import { createProviderRun, events, StubProvider } from '@demicodes/provider/testing'
import { deferred } from '@demicodes/utils'
import { ProviderAssembly } from '../llm/assembly'
import { createSessionProviderResolver } from '../llm/session-providers'
import { VendorCatalog } from '../llm/vendors'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'
import { ProviderRateLimiter } from '../usage/rate-limit'
import { ProviderVault } from '../vault/providers'

test('session provider clones retain independent state and steering while refreshing edited credentials', async () => {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  const owner = (await control.createMaster({ username: 'owner', passwordHash: '!' }))!
  const conversation = await control.createConversation(owner.id)
  const vault = new ProviderVault(control, crypto.getRandomValues(new Uint8Array(32)))
  const entry = await vault.create({ ownerUserId: owner.id, label: 'Stateful', config: { kind: 'api_key', providerType: 'stateful', apiKey: 'old' } })
  let created = 0
  const disposed: number[] = []
  const steers: Array<{ runtimeId: number; text: string }> = []
  const selections: ProviderSelection[] = []
  const assembly = new ProviderAssembly(vault, {
    stateful: {
      credential: 'api_key',
      create: ({ providerId, config }) => defineProvider({
        id: providerId,
        displayName: 'Stateful',
        createRuntime: (selection) => {
          selections.push(selection)
          const makeRuntime = (cursor = 0): AgentProvider => {
            const runtimeId = ++created
            return {
              run: () => createProviderRun((async function* () {
                if (config.kind !== 'api_key') throw new Error('Expected API-key fixture')
                yield events.text(`${config.apiKey}:${++cursor}`)
                yield events.response()
              })(), { steer: (input) => { steers.push({ runtimeId, text: input.id }) } }),
              clone: () => makeRuntime(cursor),
              dispose: () => { disposed.push(runtimeId) },
            }
          }
          return makeRuntime()
        },
      }),
    },
  }, '/unused-vault', new VendorCatalog())
  const resolve = createSessionProviderResolver({
    assembly, control, mode: 'isolated', rateLimiter: new ProviderRateLimiter(),
    hostFor: async () => { throw new Error('HTTP providers must not resolve an execution target') },
  })
  const provider = (await resolve(entry.id, { agentSessionId: conversation.id }))!
  const runtime = await providerRuntime(provider, {
    providerId: entry.id,
    model: { providerId: entry.id, model: { id: 'test', name: 'Test', contextWindow: 100_000, inputLimit: null, thinking: [], acceptedExtensions: [] }, thinking: null },
  })
  const request: InferenceRequest = {
    sessionId: conversation.id, turnId: 'turn', requestId: 'request', modelId: 'test',
    cwd: '/', systemPrompt: '', items: [], tools: [], thinking: null, cancel: new AbortController().signal,
  }
  const read = async (target: AgentProvider, steerId?: string): Promise<string> => {
    const run = target.run(request)
    const iterator = run[Symbol.asyncIterator]()
    const first = await iterator.next()
    if (steerId) await run.steer!({ id: steerId, sessionId: conversation.id, turnId: 'turn', content: [] })
    while (!(await iterator.next()).done) {}
    return !first.done && first.value.type === 'text_delta' ? first.value.text : 'missing text'
  }
  let clone: AgentProvider | undefined
  try {
    expect(await read(runtime, 'original')).toBe('old:1')
    clone = runtime.clone()
    expect(await read(clone, 'clone')).toBe('old:2')
    expect(await read(runtime)).toBe('old:2')
    expect(created).toBe(2)
    expect(steers).toEqual([{ runtimeId: 1, text: 'original' }, { runtimeId: 2, text: 'clone' }])
    await runtime.dispose?.()
    expect(await read(clone)).toBe('old:3')
    expect(disposed).toEqual([1])
    await vault.update(entry.id, { config: { kind: 'api_key', providerType: 'stateful', apiKey: 'new' } })
    assembly.invalidate(entry.id)
    expect(await read(clone)).toBe('new:1')
    expect(disposed).toEqual([1, 2])
    expect(created).toBe(3)
    request.modelId = 'next-model'
    request.thinking = { type: 'effort', effort: 'high', summary: null }
    request.serviceTierId = 'fast'
    expect(await read(clone)).toBe('new:1')
    expect(selections.at(-1)?.model).toMatchObject({ model: { id: 'next-model' }, thinking: request.thinking, serviceTierId: 'fast' })
    await vault.update(entry.id, { config: { kind: 'api_key', providerType: 'stateful', apiKey: 'third' } })
    expect(await read(clone)).toBe('third:1')
    expect(selections.at(-1)?.model).toMatchObject({ model: { id: 'next-model' }, thinking: request.thinking, serviceTierId: 'fast' })
  } finally {
    await clone?.dispose?.()
    db.close()
  }
})

test('a delayed old vault read cannot poison the cache after a provider edit', async () => {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  const owner = (await control.createMaster({ username: 'owner', passwordHash: '!' }))!
  const captured = deferred<void>()
  const release = deferred<void>()
  let pauseNextRead = false
  class PausedVault extends ProviderVault {
    override async get(id: string) {
      const entry = await super.get(id)
      if (pauseNextRead) {
        pauseNextRead = false
        captured.resolve()
        await release.promise
      }
      return entry
    }
  }
  const vault = new PausedVault(control, crypto.getRandomValues(new Uint8Array(32)))
  const entry = await vault.create({ ownerUserId: owner.id, label: 'Original', config: { kind: 'api_key', providerType: 'test', apiKey: 'old' } })
  const assembly = new ProviderAssembly(vault, {
    test: {
      credential: 'api_key',
      create: ({ providerId, label, config }) => defineProvider({
        id: providerId,
        displayName: `${label}:${config.kind === 'api_key' ? config.apiKey : ''}`,
        createRuntime: () => new StubProvider([[events.response()]]),
      }),
    },
  }, '/unused-vault', new VendorCatalog())
  try {
    pauseNextRead = true
    const oldLookup = assembly.providerFor(entry.id)
    await captured.promise
    await vault.update(entry.id, { label: 'Edited', config: { kind: 'api_key', providerType: 'test', apiKey: 'new' } })
    assembly.invalidate(entry.id)
    expect((await assembly.providerFor(entry.id))?.provider.displayName).toBe('Edited:new')
    release.resolve()
    expect((await oldLookup)?.provider.displayName).toBe('Original:old')
    const current = await assembly.providerFor(entry.id)
    expect(current?.provider.displayName).toBe('Edited:new')
    expect((await assembly.providerFor(entry.id))?.provider).toBe(current?.provider)
  } finally {
    release.resolve()
    db.close()
  }
})
