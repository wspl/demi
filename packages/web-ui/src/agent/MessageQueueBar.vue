<script setup lang="ts">
import { DeleteLine } from '@mingcute/vue/delete'
import { ListCheckLine } from '@mingcute/vue/list-check'
import { SendLine } from '@mingcute/vue/send'

defineProps<{
  messages: { id: string; text: string }[]
}>()

const emit = defineEmits<{
  remove: [id: string]
  sendNow: [id: string]
  clearAll: []
}>()
</script>

<template>
  <div class="relative mx-3 -mb-3">
    <div class="rounded-t-lg bg-surface-raised border border-line-subtle border-b-0 pb-4.5">
      <div class="flex flex-col px-1 pt-1.5">
        <div class="flex items-center gap-2 px-1.5 pb-1">
          <div class="flex flex-1 items-center gap-1.5 text-[12px] text-fg-subtle">
            <ListCheckLine :size="14" />
            <span>{{ messages.length }} Queued</span>
          </div>
          <div class="shrink-0 flex items-center gap-0.5">
            <span
              class="flex size-6 cursor-pointer items-center justify-center rounded-md text-fg-subtle transition-colors hover:bg-active hover:text-fg-body"
              @click="emit('clearAll')"
            >
              <DeleteLine :size="14" />
            </span>
          </div>
        </div>
        <div class="flex flex-col">
          <div
            v-for="(message, index) in messages"
            :key="message.id"
            class="flex items-center gap-2 px-1.5"
          >
            <span class="shrink-0 text-[12px] tabular-nums text-fg-faint">{{ index + 1 }}</span>
            <div
              class="flex min-w-0 flex-1 items-center gap-2 py-1"
              :class="index < messages.length - 1 ? 'border-b border-line-subtle' : ''"
            >
              <span class="min-w-0 flex-1 truncate text-[13px] text-fg-muted">{{ message.text }}</span>
              <div class="shrink-0 flex items-center gap-0.5">
                <span
                  class="flex size-6 cursor-pointer items-center justify-center rounded-md text-fg-subtle transition-colors hover:bg-active hover:text-fg-body"
                  @click="emit('sendNow', message.id)"
                >
                  <SendLine :size="14" />
                </span>
                <span
                  class="flex size-6 cursor-pointer items-center justify-center rounded-md text-fg-subtle transition-colors hover:bg-active hover:text-fg-body"
                  @click="emit('remove', message.id)"
                >
                  <DeleteLine :size="14" />
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
