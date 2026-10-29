// The reply's appearance as a widget's page draws in it: the frame's name
// gives its bootstrap the root's appearance attributes, its font and its text
// size (`crates/plugin-widget/plugin/src/bootstrap.js`).
import { onBeforeUnmount, onMounted, ref, type Ref } from 'vue'

/** The root's attributes the appearance stylesheet reads. */
const LOOK_ATTRIBUTES = ['data-theme', 'data-tone', 'data-accent', 'data-density', 'data-radius', 'data-shadow', 'lang']

/** The appearance now, as the frame's name hands it to the page. */
function readLook(): string {
  const root = document.documentElement
  const style = getComputedStyle(root)
  const attributes = Object.fromEntries(LOOK_ATTRIBUTES.flatMap((name) => {
    const value = root.getAttribute(name)
    return value === null ? [] : [[name, value]]
  }))
  const properties = {
    '--font-sans': style.getPropertyValue('--font-sans').trim(),
    '--widget-text': style.getPropertyValue('--agent-text').trim(),
  }
  return JSON.stringify({ attributes, properties })
}

/** The appearance, which follows each change of it while the component is mounted. */
export function useLook(): Ref<string> {
  const look = ref(readLook())
  let watcher: MutationObserver | null = null
  onMounted(() => {
    watcher = new MutationObserver(() => {
      const next = readLook()
      if (next !== look.value)
        look.value = next
    })
    watcher.observe(document.documentElement, { attributes: true, attributeFilter: LOOK_ATTRIBUTES.concat('style') })
  })
  onBeforeUnmount(() => watcher?.disconnect())
  return look
}
