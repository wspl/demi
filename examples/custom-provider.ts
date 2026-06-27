// Implementing the #1 extension point — a provider — from scratch.
//
//   bun run examples/custom-provider.ts
//
// This adapts an arbitrary backend (here a trivial deterministic one) to the
// @demicodes/provider contract and drives it through the same runtime + client the
// REPL and web UI use, proving a custom provider composes end-to-end.
import { AgentServer } from '@demicodes/agent'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import { zeroUsage, type Block } from '@demicodes/core'
import { defineProvider, modelSelectionFromCatalog } from '@demicodes/provider'
import type { AgentProvider, InferenceRequest, Provider, ProviderEvent, ProviderRun } from '@demicodes/provider'

/**
 * A from-scratch provider. The only required pieces are an id/displayName and a
 * `createRuntime()` returning something with `run(request) -> ProviderRun` (an
 * async iterable of ProviderEvents). A real adapter would call an API or CLI here
 * and map its stream onto ProviderEvents; this one just echoes the last user text.
 */
export function createScriptedProvider(): Provider {
  return defineProvider({
    id: 'scripted',
    displayName: 'Scripted',
    createRuntime(): AgentProvider {
      return {
        run(request: InferenceRequest): ProviderRun {
          async function* events(): AsyncGenerator<ProviderEvent> {
            const lastUser = [...request.items].reverse().find((item) => item.type === 'user_message')
            const text =
              lastUser?.type === 'user_message'
                ? lastUser.content.map((block) => (block.type === 'text' ? block.text : '')).join('')
                : ''
            yield { type: 'text_delta', text: `echo: ${text}` }
            // A terminal `response` event (with token usage) ends the turn.
            yield { type: 'response', usage: zeroUsage() }
          }
          return events()
        },
      }
    },
  })
}

async function main(): Promise<void> {
  const cwd = process.cwd()
  const host = new LocalHost(cwd)
  const harness = createCodingAgentHarness({ host })
  const server = new AgentServer({ agent: harness, providers: [createScriptedProvider()] })
  const client = server.client()

  // Each patch carries the *full* current text per block; print only the new suffix
  // (keyed by block id) so streaming renders correctly instead of cumulatively.
  const printed = new Map<string, number>()
  function render(blocks: Block[]): void {
    for (const block of blocks) {
      if (block.type !== 'text') continue
      const already = printed.get(block.id) ?? 0
      if (block.text.length > already) {
        process.stdout.write(block.text.slice(already))
        printed.set(block.id, block.text.length)
      }
    }
  }

  let started = false
  const done = new Promise<void>((resolve) => {
    const unsubscribe = client.subscribe((event) => {
      if (event.type === 'transcript_snapshot' || event.type === 'transcript_patch') {
        render(event.blocks)
      } else if (event.type === 'phase') {
        if (event.phase === 'running') started = true
        else if (event.phase === 'idle' && started) {
          unsubscribe()
          resolve()
        }
      }
    })
  })

  // No catalog needed — build a selection directly for the custom model id.
  const selection = modelSelectionFromCatalog('scripted', null, { modelId: 'scripted-1', fallbackName: 'Scripted' })
  await client.open({ providerId: 'scripted', model: selection }, cwd)
  await client.send([{ type: 'text', text: 'hello from a custom provider' }])

  await done
  await client.close()
}

void main()
