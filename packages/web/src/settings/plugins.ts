import { t } from './locales/text'
import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { errorCode, reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody } from '../api/client'
import type { PluginSwitch } from '../api/generated/web-api'
import { useProduct, withPart } from '../state/product'

/**
 * The user's plugin switches. A switch shows at once, and a plugin turned off leaves the pages with
 * its state; its write follows the earlier ones of the same plugin, and a refusal turns the switch
 * back with a toast.
 */
export const usePluginSettings = defineStore('plugin-settings', () => {
  const product = useProduct()
  /** The switches on their way, by plugin: each plugin's go in order. */
  const writes = new SerialQueue()

  function switchPlugin(id: string, enabled: boolean): Promise<void> {
    const change = product.change('plugins', (state) => withPart(state, {
      type: 'plugins',
      plugins: state.plugins.map((plugin) => plugin.id === id ? { ...plugin, enabled } : plugin),
    }))
    return writes
      .run(id, async () => {
        change.send()
        await apiRequest(`/plugins/${encodeURIComponent(id)}`, {
          method: 'PUT',
          ...jsonBody({ enabled } satisfies PluginSwitch),
        })
        change.land()
      })
      .catch((error) => {
        change.drop()
        // A plugin gone from the server meanwhile has nothing to switch,
        // and the channel takes it off the list.
        if (errorCode(error) === 'unknown_plugin') {
          return
        }
        reportError(enabled ? t('plugins.turnOnFailed') : t('plugins.turnOffFailed'), error, {
          userVisible: true,
        })
      })
  }

  return { switchPlugin }
})
