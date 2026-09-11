import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { z } from 'zod'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import { preferencesSchema, type PreferencesPatch } from '../api/contracts'
import { useProduct } from './product'

export const DEFAULT_KEYS = [
  {
    id: 'new',
    action: 'New conversation',
    keys: '⌘N',
  },
  {
    id: 'sidebar',
    action: 'Toggle sidebar',
    keys: '⌘B',
  },
  {
    id: 'settings',
    action: 'Open settings',
    keys: '⌘,',
  },
] as const

export const usePreferences = defineStore('preferences', () => {
  const product = useProduct()
  const pending = ref<PreferencesPatch>({})
  const writes = new SerialQueue()
  let timer: ReturnType<typeof setTimeout> | null = null
  let controller = new AbortController()

  const appearance = computed(() => ({
    theme: 'system' as const,
    tone: 'ink' as const,
    accent: 'blue' as const,
    fontSize: 15,
    ...product.snapshot?.preferences.appearance,
    ...pending.value.appearance,
  }))
  const keys = computed(() =>
    DEFAULT_KEYS.map((binding) => {
      const queued = pending.value.shortcuts?.[binding.id]
      return {
        ...binding,
        keys:
          queued === null
            ? binding.keys
            : (queued ??
              product.snapshot?.preferences.shortcuts[binding.id] ??
              binding.keys),
      }
    }),
  )

  function update(patch: PreferencesPatch): void {
    pending.value = {
      appearance: {
        ...pending.value.appearance,
        ...patch.appearance,
      },
      shortcuts: {
        ...pending.value.shortcuts,
        ...patch.shortcuts,
      },
    }
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = setTimeout(() => {
      timer = null
      void flush()
    }, 200)
  }

  async function flush(): Promise<void> {
    const current = controller
    await writes.run(async () => {
      if (current.signal.aborted) {
        return
      }
      const patch = {
        appearance: { ...pending.value.appearance },
        shortcuts: { ...pending.value.shortcuts },
      }
      if (
        !Object.keys(patch.appearance).length &&
        !Object.keys(patch.shortcuts).length
      ) {
        return
      }
      try {
        const response = await apiRequest('/settings/preferences', {
          method: 'PATCH',
          ...jsonBody(patch),
          signal: current.signal,
        })
        await readResponse(response, z.object({ preferences: preferencesSchema }))
        await product.refresh()
      } catch (error) {
        if (!current.signal.aborted) {
          reportError('Could not update settings', error, { userVisible: true })
        }
      } finally {
        if (!current.signal.aborted) {
          for (const field of ['appearance', 'shortcuts'] as const) {
            const queued = pending.value[field] as Record<string, unknown>
            for (const [key, value] of Object.entries(patch[field])) {
              if (queued?.[key] === value) {
                delete queued[key]
              }
            }
          }
        }
      }
    })
  }

  function stop(): void {
    controller.abort()
    controller = new AbortController()
    if (timer !== null) {
      clearTimeout(timer)
    }
    timer = null
    pending.value = {}
  }

  return {
    appearance,
    keys,
    update,
    flush,
    stop,
  }
})
