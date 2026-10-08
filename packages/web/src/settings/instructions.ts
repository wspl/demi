import { computed } from 'vue'
import { defineStore } from 'pinia'
import { apiRequest, jsonBody } from '../api/client'
import type { InstructionsBody } from '../api/generated/web-api'
import { useProduct } from '../state/product'

/**
 * The user's personal instructions as the Instructions section saves them
 * (`web-api.md` § Instructions). A save's answer goes into the product state
 * at once; the channel brings the same part to every other page.
 */
export const useInstructionSettings = defineStore('instruction-settings', () => {
  const product = useProduct()
  const text = computed(() => product.snapshot?.instructions ?? null)

  /** Saves `next`; a failure reaches the section, which shows it. */
  async function save(next: string): Promise<void> {
    const at = product.sent()
    await apiRequest('/instructions', {
      method: 'PUT',
      ...jsonBody({ text: next } satisfies InstructionsBody),
    })
    product.answered(at, { type: 'instructions', instructions: next.trim() === '' ? '' : next })
  }

  return { text, save }
})
