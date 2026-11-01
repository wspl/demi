// Whether an element is on screen: in the viewport of a page in the
// foreground. The demos and the endless animations stop while theirs is not,
// so a page scrolled past or left in a background tab costs no work.
import { onBeforeUnmount, onMounted, ref, type Ref } from 'vue'

/** Whether `target` is on screen, false until it mounts. */
export function useOnScreen(target: Readonly<Ref<HTMLElement | null>>): Readonly<Ref<boolean>> {
  const onScreen = ref(false)
  let inView = false
  const update = (): void => {
    onScreen.value = inView && document.visibilityState === 'visible'
  }
  let observer: IntersectionObserver | undefined
  onMounted(() => {
    observer = new IntersectionObserver((entries) => {
      inView = entries.at(-1)!.isIntersecting
      update()
    })
    observer.observe(target.value!)
    document.addEventListener('visibilitychange', update)
  })
  onBeforeUnmount(() => {
    observer?.disconnect()
    document.removeEventListener('visibilitychange', update)
  })
  return onScreen
}
