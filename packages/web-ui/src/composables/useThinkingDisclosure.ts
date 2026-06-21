import { ref, watch, type Ref } from 'vue'

export function useThinkingDisclosure(options: {
  hasContent: () => boolean
  isStreaming: () => boolean
}): Ref<boolean> {
  const isOpen = ref(options.isStreaming() && options.hasContent())

  watch(
    [options.hasContent, options.isStreaming],
    ([hasContent, isStreaming], [hadContent, wasStreaming]) => {
      if (isStreaming && hasContent && (!hadContent || !wasStreaming)) {
        isOpen.value = true
      }
    },
  )

  return isOpen
}
