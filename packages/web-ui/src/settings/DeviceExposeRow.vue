<script setup lang="ts">
import { computed } from 'vue'
import Button from '../ui/Button.vue'
import CopyCode from '../ui/CopyCode.vue'
import ExternalLink from '../ui/ExternalLink.vue'
import SettingsRow from './SettingsRow.vue'
import Tag from '../ui/Tag.vue'
import { formatTimeRemaining, useTimeRemaining } from '../composables/useRelativeTime'
import type { SettingsExpose } from './types'

const props = defineProps<{
  expose: SettingsExpose
  pending?: boolean
}>()
const emit = defineEmits<{
  renew: []
  remove: []
}>()
const remaining = useTimeRemaining(() => props.expose.expiresAt)
const label = computed(() => formatTimeRemaining(remaining.value))
const expired = computed(() => remaining.value <= 0)
const expiresAt = computed(() => new Date(props.expose.expiresAt).toLocaleString())
</script>

<template>
  <SettingsRow inset :label="expose.address">
    <template #tags>
      <Tag :tone="expired ? 'danger' : remaining < 60_000 ? 'warning' : 'neutral'">
        <time :datetime="expose.expiresAt" :title="`Expires ${expiresAt}`">{{ label }}</time>
      </Tag>
    </template>
    <template #detail>
      <div class="flex min-w-0 items-center gap-3">
        <div class="min-w-0 flex-1">
          <CopyCode :code="expose.url" copy-label="Copy URL" truncate />
        </div>
        <ExternalLink :href="expose.url">Open</ExternalLink>
      </div>
    </template>
    <Button size="sm" :loading="pending" @click="emit('renew')">Renew</Button>
    <Button size="sm" :loading="pending" @click="emit('remove')">Remove</Button>
  </SettingsRow>
</template>
