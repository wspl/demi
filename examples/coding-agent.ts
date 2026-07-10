// A minimal Demi coding agent driven through an in-process client.
//
//   bun run examples/coding-agent.ts
//
// It assembles a Host, a coding harness, and a provider, then sends one turn and
// streams the assistant's text to stdout.
import { AgentServer } from '@demicodes/agent'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import type { Block } from '@demicodes/core'
import { modelSelectionFromCatalog } from '@demicodes/provider'
import { createClaudeCodeProvider, listClaudeCodeModels } from '@demicodes/provider-claude-code'

async function main(): Promise<void> {
  const cwd = process.cwd()

  // A Host gives the agent a filesystem, process spawning, and a scoped store.
  const host = new LocalHost(cwd)

  // The harness supplies the system prompt, registered commands, and reference resolution.
  const harness = createCodingAgentHarness({ host })

  // One or more inference providers.
  const providers = [createClaudeCodeProvider()]

  // The server owns the session lifecycle; an in-process client speaks the same
  // protocol the REPL and web UI use.
  const server = new AgentServer({
    agent: harness,
    providers,
    shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
  })
  const client = server.client()

  // Each patch carries the full current text per block; print only the new suffix
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

  // Resolve once the turn we send has run to completion (running -> idle).
  let started = false
  const done = new Promise<void>((resolve) => {
    const unsubscribe = client.subscribe((event) => {
      if (event.type === 'transcript_reset' || event.type === 'transcript_patch') {
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

  // Pick the provider's default model and turn the catalog entry into a selection.
  const catalog = await listClaudeCodeModels()
  const model = catalog.models.find((m) => m.id === catalog.defaultModelId) ?? catalog.models[0] ?? null
  const selection = modelSelectionFromCatalog('claude-code', model)

  await client.open({ providerId: 'claude-code', model: selection }, cwd, globalThis.crypto.randomUUID())
  await client.send([{ type: 'text', text: 'Create hello.txt containing "hi", then read it back.' }])

  await done
  await client.close()
}

void main()
