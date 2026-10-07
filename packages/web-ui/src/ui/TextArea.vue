<script setup lang="ts">
import { computed, onMounted, ref, useAttrs, watch } from 'vue'
import { useOverlayScrollbars } from 'overlayscrollbars-vue'
import { keepComposition } from '@demicodes/utils'
import { disabledTooltip } from './disabled'
import { scrollbarsOptions, scrollbarsTarget } from './scrollbars'
import Tooltip from './Tooltip.vue'
import { useAutofocus } from './autofocus'

defineOptions({ inheritAttrs: false })

/**
 * Text of several lines, such as instructions a user writes: the frame of
 * `TextInput` around a field that keeps its line breaks. It is `rows` lines
 * tall and scrolls past them; the user can make it taller.
 */
const props = withDefaults(defineProps<{
  modelValue?: string
  placeholder?: string
  rows?: number
  /** Take focus on mount: the field a dialog or form opens on. */
  focused?: boolean
  /** Text in a fixed-width face, such as a prompt or a script. */
  mono?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
}>(), {
  rows: 6,
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

// Layout classes belong to the frame; native attributes stay on the field.
const attrs = useAttrs()
const layoutAttrs = computed(() => ({
  class: attrs['class'],
  style: attrs['style'],
}))
const fieldAttrs = computed(
  () => Object.fromEntries(
    Object.entries(attrs).filter(([key]) => key !== 'class' && key !== 'style')
  )
)
const fieldRef = ref<HTMLTextAreaElement>()
const frameRef = ref<HTMLElement>()
const isFocused = ref(false)
const autofocus = useAutofocus()
const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))

function input(event: Event): void {
  emit('update:modelValue', (event.target as HTMLTextAreaElement).value)
}

// The field scrolls past its rows with the app's scrollbar, in its frame.
const [initializeScrollbars, scrollbars] = useOverlayScrollbars({ options: scrollbarsOptions('y') })
// A field's text is no change OverlayScrollbars observes; it measures again for each new value.
watch(() => props.modelValue, () => scrollbars()?.update(true), { flush: 'post' })

onMounted(() => {
  if (fieldRef.value && frameRef.value)
    initializeScrollbars(scrollbarsTarget(fieldRef.value, frameRef.value))
  if (props.focused)
    autofocus(fieldRef.value)
})

defineExpose({
  focus() {
    fieldRef.value?.focus()
  },
})
</script>

<template>
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="div"
    v-bind="layoutAttrs"
    data-field
    class="flex min-w-0"
    :class="attrs['class'] ? '' : 'w-full'"
    :open-delay-ms="80"
  >
    <!-- An input method's keys stay with the field: a candidate's Enter submits nothing. -->
    <div
      ref="frameRef"
      class="relative flex min-w-0 flex-1 rounded-md bg-surface-raised ring-1 transition-[box-shadow] duration-200 ease-out"
      :class="[
        isFocused ? 'ring-line-focus' : 'ring-line',
        disabled ? 'cursor-not-allowed opacity-40' : '',
      ]"
      @keydown.capture="keepComposition"
    >
      <textarea
        ref="fieldRef"
        data-overlayscrollbars-initialize
        v-bind="fieldAttrs"
        :value="modelValue"
        :rows="rows"
        :placeholder="placeholder"
        :disabled="disabled || undefined"
        class="min-h-0 w-full min-w-0 resize-y bg-transparent px-2.5 py-1.5 text-chrome leading-5 text-fg outline-none placeholder:text-fg-subtle"
        :class="mono ? 'font-mono text-[12px]' : ''"
        @input="input"
        @focus="isFocused = true"
        @blur="isFocused = false"
      />
    </div>
  </Tooltip>
</template>
