/**
 * The `mobile` page's store (`packages/web-ui/src/plugins/stores.ts`): each
 * conversation's simulators on the page (`./controller.ts`), kept while the
 * plugin is on, by conversation, each device on a stream of its own.
 *
 * A device tab's content watches its conversation's simulators while it is
 * mounted. One no content watches is kept, with each device's stream, its
 * canvas and its look, so a return to the conversation shows them at once;
 * past the limit, the least recently watched is released.
 */
import { effectScope, onScopeDispose } from 'vue'
import { KeptSessions, definePluginStore, type KeptSession, type PageContext, type PluginStore } from '@demicodes/plugin-sdk'
import { mobileApi } from '../devices'
import { MobileDevicesController, type MobileControllerOptions } from './controller'

/** One conversation's simulators as the store keeps them. */
export interface KeptDevices extends KeptSession {
  readonly devices: MobileDevicesController
}

export type MobileStore = PluginStore<KeptSessions<KeptDevices>>

/** The store of a page whose simulators take `options`. */
export function mobileStore(options: MobileControllerOptions): MobileStore {
  return definePluginStore('devices', (page) => {
    const devices = new KeptSessions((conversation) => keptDevices(page, conversation, options))
    // The store ends as its plugin is turned off.
    onScopeDispose(() => devices.dispose())
    return devices
  })
}

function keptDevices(page: PageContext, conversation: string, options: MobileControllerOptions): KeptDevices {
  // Its own scope, which ends when the store releases it rather than with whatever made it.
  const scope = effectScope(true)
  const devices = scope.run(() => new MobileDevicesController(
    mobileApi(page.plugin.conversation(conversation)),
    page.errors,
    { cache: page.cache, ...options },
  ))!
  return {
    devices,
    // Whether each device's stream is looked at follows its views (`DeviceSession.showIn`).
    watched: () => {},
    dispose: () => {
      devices.dispose()
      scope.stop()
    },
  }
}
