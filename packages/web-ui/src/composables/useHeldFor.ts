import { onScopeDispose, shallowRef, watch, type Ref } from 'vue'

/**
 * Whether `on` has held for `ms` without a break: true once it has, false the
 * moment it ends, so a wait that passes on its own shows nothing.
 */
export function useHeldFor(on: () => boolean, ms: number): Readonly<Ref<boolean>> {
  const held = shallowRef(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  watch(on, (now) => {
    clearTimeout(timer)
    held.value = false
    if (now) {
      timer = setTimeout(() => {
        held.value = true
      }, ms)
    }
  }, { immediate: true })
  onScopeDispose(() => clearTimeout(timer))
  return held
}
