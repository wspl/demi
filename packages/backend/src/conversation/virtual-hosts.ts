import { join } from 'node:path'
import { VirtualHost, scopedFsBackend } from '@demicodes/host-virtual'
import type { HostFileSystem } from '@demicodes/shell'
import type { ConversationStores } from '../storage/conversation-store'
import { HOSTLESS_HOME, HOSTLESS_NAMESPACE } from './scoped-transport'

/**
 * One stable VirtualHost per conversation — `AgentHarness.host` must return
 * the same object for the same execution target (per-Host shell reuse). The
 * host's store is that conversation's own database scope.
 */
export function createVirtualHostFactory(options: {
  dataDir: string
  conversationStores: ConversationStores
  localFs: HostFileSystem
}): (conversationId: string) => Promise<VirtualHost> {
  const virtualHosts = new Map<string, Promise<VirtualHost>>()
  return (conversationId) => {
    let host = virtualHosts.get(conversationId)
    if (!host) {
      host = (async () => {
        const virtual = new VirtualHost({
          backend: scopedFsBackend(join(options.dataDir, 'virtual', conversationId), options.localFs),
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
