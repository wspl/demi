import type { Block, ProviderFailureFacts } from '@demicodes/core'
import type { ProviderFailureReader } from '@demicodes/provider'
import type { ProviderAssembly } from '../llm/assembly'

/** The facts of a transcript's error blocks, by block id, as the browser gets them. */
export type FailureFacts = Record<string, ProviderFailureFacts>

/** Reads the failure facts of a list of blocks; the one function both delivery paths call. */
export type FailureFactsReader = (blocks: readonly Block[]) => Promise<FailureFacts>

/**
 * The failure facts of a list of blocks (`docs/demi-next/backend.md` § Failure
 * facts): each error block with a record is read by the provider named in its
 * model selection. A block whose provider configuration is gone, or whose
 * provider reads nothing, yields no entry.
 */
export function failureFactsReader(assembly: ProviderAssembly): FailureFactsReader {
  return async (blocks) => {
    const failures: FailureFacts = {}
    const readers = new Map<string, Promise<ProviderFailureReader | null>>()
    for (const block of blocks) {
      if (block.type !== 'error' || block.diagnostics?.upstream === undefined)
        continue
      const providerId = block.model.providerId
      if (!readers.has(providerId))
        readers.set(providerId, readerOf(assembly, providerId))
      const read = await readers.get(providerId)!
      if (read)
        failures[block.id] = read(block.diagnostics, block.createdAt)
    }
    return failures
  }
}

/**
 * The reader of a configured provider. Facts only add to a record the browser
 * gets anyway, so a configuration that cannot be read yields no reader rather
 * than holding back the transcript.
 */
async function readerOf(
  assembly: ProviderAssembly,
  providerId: string
): Promise<ProviderFailureReader | null> {
  try {
    return (await assembly.providerFor(providerId))?.provider.readFailure ?? null
  } catch {
    return null
  }
}
