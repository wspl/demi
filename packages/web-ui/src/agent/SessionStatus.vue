<script setup lang="ts">
import { computed } from 'vue'
import { t } from '../infra/i18n'
import RegionStatus from '../ui/RegionStatus.vue'
import { sessionStatusCopy, type SessionStatusKind } from './session-status'

/**
 * The session pane's stand-in for the transcript: loading, failed, empty or
 * missing. It is the shared `RegionStatus` pane over the session copy table,
 * filling the pane it replaces.
 */
const props = defineProps<{
  kind: SessionStatusKind
  /** A failed restore names the reason under the label. */
  detail?: string | null
}>()

const emit = defineEmits<{
  retry: []
  create: []
}>()

const copy = computed(() => sessionStatusCopy(props.kind))
const actionLabel = computed(() => {
  if (copy.value.action === 'retry') {
    return t('agent.session.retry')
  }
  if (copy.value.action === 'create') {
    return t('agent.session.new')
  }
  return undefined
})

function act(): void {
  if (copy.value.action === 'retry') {
    emit('retry')
  } else if (copy.value.action === 'create') {
    emit('create')
  }
}
</script>

<template>
  <RegionStatus
    class="h-full min-h-0 flex-1"
    :busy="kind === 'loading'"
    :failed="kind === 'failed'"
    :label="copy.label"
    :detail="kind === 'failed' ? detail : null"
    :action="actionLabel"
    @action="act"
  />
</template>
