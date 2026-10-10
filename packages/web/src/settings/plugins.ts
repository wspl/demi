import { defineStore } from 'pinia'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody } from '../api/client'
import type { PluginSwitch } from '../api/generated/web-api'
import { useProduct, withPart } from '../state/product'

/**
 * The user's plugin switches (`web-api.md` § A user's plugins). A switch
 * shows at once, and a plugin turned off leaves the pages with its state;
 * its write follows the earlier ones (`web-application.md` § Responding to
 * the user), and a refusal turns the switch back with a toast.
 */
export const usePluginSettings = defineStore('plugin-settings', () => {
  const product = useProduct()
  const writes = new SerialQueue()

  function switchPlugin(id: string, enabled: boolean): Promise<void> {
    const change = product.change('plugins', (state) => withPart(state, {
      type: 'plugins',
      plugins: state.plugins.map((plugin) => plugin.id === id ? { ...plugin, enabled } : plugin),
    }))
    return writes
      .run(async () => {
        change.send()
        await apiRequest(`/plugins/${encodeURIComponent(id)}`, {
          method: 'PUT',
          ...jsonBody({ enabled } satisfies PluginSwitch),
        })
        change.land()
      })
      .catch((error) => {
        change.drop()
        reportError(enabled ? 'Could Not Turn the Plugin On' : 'Could Not Turn the Plugin Off', error, {
          userVisible: true,
        })
      })
  }

  return { switchPlugin }
})
