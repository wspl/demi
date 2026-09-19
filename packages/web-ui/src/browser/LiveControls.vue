<script setup lang="ts">
import { computed, ref } from 'vue'
import type { LiveControl, LiveViewport } from '@demicodes/browser-protocol/live'
import type { LiveSession } from './session'
import { panelRect, type Placement } from './view'

/**
 * The watched page's native form controls, as the viewer's own controls over
 * the picture (`browser-live-view.md` § Input): a select, a date, a time, a
 * colour, a suggestion field or a file input opens the viewer's own picker,
 * and the choice goes back to the control the viewer saw.
 */
const props = defineProps<{
  session: LiveSession
  controls: readonly LiveControl[]
  placement: Placement
  viewport: LiveViewport
}>()

/** A list without a popup shows its own pixels; the page's input stays. */
const overlaid = computed(() => props.controls.filter(
  (control) => !(control.kind === 'select' && (control.multiple || control.size > 1)),
))

const editing = ref<string | null>(null)

function style(control: LiveControl): Record<string, string> {
  const rect = panelRect(control.rect, props.placement)
  return {
    left: `${rect.left}px`,
    top: `${rect.top}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
    fontSize: `${14 * props.placement.scale}px`,
  }
}

function chooseOption(control: LiveControl, event: Event): void {
  const select = event.target as HTMLSelectElement
  const indices = [...select.options].flatMap((option, index) => option.selected ? [index] : [])
  props.session.choose(control, select.value, indices)
}

function chooseValue(control: LiveControl, event: Event): void {
  props.session.choose(control, (event.target as HTMLInputElement).value, [])
}

function chooseFiles(control: LiveControl, event: Event): void {
  const files = [...((event.target as HTMLInputElement).files ?? [])]
  if (files.length > 0) {
    void props.session.upload(control, files)
  }
}

/** A date, time or colour opens its picker where the viewer clicked it. */
function openPicker(event: MouseEvent): void {
  const input = event.target as HTMLInputElement & { showPicker?: () => void }
  if (typeof input.showPicker === 'function') {
    event.preventDefault()
    input.showPicker()
  }
}
</script>

<template>
  <div class="pointer-events-none absolute inset-0">
    <template v-for="control in overlaid" :key="control.token">
      <select
        v-if="control.kind === 'select'"
        class="pointer-events-auto absolute m-0 box-border rounded border border-line bg-transparent p-0 px-1 text-transparent opacity-0"
        :aria-label="control.label"
        :disabled="control.disabled"
        :required="control.required"
        :style="style(control)"
        @change="chooseOption(control, $event)"
      >
        <option
          v-for="(option, index) in control.options"
          :key="index"
          :value="option.value"
          :disabled="option.disabled"
          :hidden="option.hidden"
          :selected="option.selected"
          :label="option.group ? `${option.group} — ${option.label}` : option.label"
        >
          {{ option.label }}
        </option>
      </select>
      <template v-else-if="control.kind === 'file'">
        <input
          type="file"
          class="pointer-events-auto absolute m-0 box-border opacity-0"
          :aria-label="control.label"
          :accept="control.accept || undefined"
          :multiple="control.multiple"
          :disabled="control.disabled"
          :style="style(control)"
          @change="chooseFiles(control, $event)"
        >
      </template>
      <input
        v-else
        :type="control.kind === 'suggestions' ? 'text' : control.kind"
        class="pointer-events-auto absolute m-0 box-border rounded border border-line bg-transparent p-0 px-1 text-transparent opacity-0"
        :aria-label="control.label"
        :value="editing === control.token && control.kind === 'suggestions' ? undefined : control.value"
        :min="control.min || undefined"
        :max="control.max || undefined"
        :step="control.step || undefined"
        :disabled="control.disabled"
        :required="control.required"
        :list="control.kind === 'suggestions' ? `live-${control.token}` : undefined"
        :style="style(control)"
        @focus="editing = control.token"
        @blur="editing = editing === control.token ? null : editing"
        @click="control.kind === 'suggestions' ? undefined : openPicker($event)"
        @change="chooseValue(control, $event)"
        @input="control.kind === 'suggestions' ? chooseValue(control, $event) : undefined"
      >
      <datalist v-if="control.kind === 'suggestions'" :id="`live-${control.token}`">
        <option
          v-for="(option, index) in control.options"
          :key="index"
          :value="option.value"
          :label="option.label"
        />
      </datalist>
    </template>
  </div>
</template>
