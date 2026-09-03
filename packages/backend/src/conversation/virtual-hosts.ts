import { VirtualHost, type VirtualFsBackend } from '@demicodes/host-virtual'
import { HOSTLESS_HOME, HOSTLESS_NAMESPACE } from './scoped-transport'
import type { ConversationStores } from '../storage/conversation-store'

/**
 * One stable VirtualHost per conversation — `AgentHarness.host` must return
 * the same object for the same execution target (per-Host shell reuse). The
 * host's files are that conversation's `files` tree and its store is that
 * conversation's own database scope.
 */
export function createVirtualHostFactory(options: {
  conversationStores: ConversationStores
  /** The filesystem behind a conversation's namespace; the default is its files tree. */
  backendFor?: (conversationId: string) => VirtualFsBackend
}): (conversationId: string) => Promise<VirtualHost> {
  const virtualHosts = new Map<string, Promise<VirtualHost>>()
  const backendFor = options.backendFor ?? ((conversationId: string) => options.conversationStores.filesBackend(conversationId))
  return (conversationId) => {
    let host = virtualHosts.get(conversationId)
    if (!host) {
      host = (async () => {
        const virtual = new VirtualHost({
          backend: backendFor(conversationId),
          store: options.conversationStores.hostStore(conversationId),
          defaultCwd: HOSTLESS_HOME,
          directories: HOSTLESS_NAMESPACE,
        })
        await virtual.ensureLayout()
        return virtual
      })()
      virtualHosts.set(conversationId, host)
    }
    return host
  }
}
