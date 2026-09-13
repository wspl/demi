<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '../../ui/icon-metrics'
import type { ToolCallBlock } from '../block-types'
import { storedShellView } from '../block-helpers'
import { useEditSelection } from '../edit-selection'
import FileChangePills from './FileChangePills.vue'

const props = defineProps<{ block: ToolCallBlock }>()
const call = computed(() => storedShellView(props.block))
const select = useEditSelection()

function pick(path: string): void {
  const current = call.value
  const file = current?.files?.find((entry) => entry.path === path)
  if (current && file) {
    select?.({ commandId: current.commandId, file })
  }
}
</script>

<template>
  <FileChangePills
    v-if="call?.files && call.files.length > 0"
    class="py-1"
    :style="{ paddingLeft: `${ICON_PX.in28 + 8}px` }"
    :files="call.files"
    @select="pick"
  />
</template>
