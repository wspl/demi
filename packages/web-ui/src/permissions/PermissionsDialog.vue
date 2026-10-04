<script setup lang="ts">
import Button from '../ui/Button.vue'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Dialog from '../ui/Dialog.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { categoryTitle, type PermissionGrantView } from './types'

/**
 * The conversation's grants, from Permissions in its sidebar menu
 * (`permissions.md` § Grants): each category the user allowed, with what it
 * allows, when it was granted and Revoke. Without grants it says that Demi
 * asks when an agent first needs a permission.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The conversation's title, which the dialog names. */
  title: string
  grants: readonly PermissionGrantView[]
  load?: 'loading' | 'ready' | 'failed'
  /** The categories whose revocation is on its way. */
  revoking?: readonly string[]
}>()

const emit = defineEmits<{
  close: []
  revoke: [category: string]
  retry: []
}>()
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Permissions"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 p-5">
      <header class="flex flex-col gap-1 pr-8">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Permissions</h3>
        <p class="truncate text-[13px] text-fg-muted">{{ title }}</p>
      </header>
      <AsyncRegion :state="load ?? 'ready'" @retry="emit('retry')">
        <ul v-if="grants.length" class="flex flex-col divide-y divide-line-subtle">
          <li
            v-for="grant in grants"
            :key="grant.category.id"
            class="flex items-start gap-3 py-3 first:pt-0 last:pb-0"
          >
            <div class="flex min-w-0 flex-1 flex-col gap-1">
              <span class="text-chrome font-medium text-fg-emphasis">{{
                categoryTitle(grant.category)
              }}</span>
              <p
                v-if="grant.category.description"
                class="text-[13px] leading-5 text-fg-muted"
              >
                {{ grant.category.description }}
              </p>
              <span class="text-[12px] text-fg-subtle">
                Allowed <RelativeTime :timestamp="grant.grantedAt" />
              </span>
            </div>
            <Button
              size="sm"
              class="shrink-0"
              :loading="revoking?.includes(grant.category.id)"
              @click="emit('revoke', grant.category.id)"
              >Revoke</Button
            >
          </li>
        </ul>
        <p v-else class="py-2 text-[13px] leading-5 text-fg-subtle">
          This conversation has no permissions. Demi asks you when an agent
          first needs one.
        </p>
      </AsyncRegion>
    </div>
  </Dialog>
</template>
