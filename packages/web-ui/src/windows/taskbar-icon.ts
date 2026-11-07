import { h, type Component, type FunctionalComponent } from 'vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { TaskbarItem, TaskbarTab } from './Taskbar.vue'

/** Each page picture drawn as an icon, by its URL, so a render keeps the same image. */
const pictures = new Map<string, Component>()

/** A tab's mark as an icon: its page's picture, else its kind's mark, else the window's icon. */
export function tabIcon(tab: TaskbarTab | undefined, icon: Component): Component {
  const picture = tab?.icon
  if (!picture) {
    return tab?.mark ?? icon
  }
  let drawn = pictures.get(picture)
  if (!drawn) {
    const image: FunctionalComponent<{ size?: number }> = ({ size = ICON_PX.in28 }) =>
      h('img', { src: picture, alt: '', width: size, height: size, class: 'shrink-0 rounded-sm', draggable: false })
    image.props = ['size']
    drawn = image
    pictures.set(picture, drawn)
  }
  return drawn
}

/**
 * What a window shows itself by on the taskbar or the phone's list of
 * windows: its shown tab's own icon, as the tab strip draws it, such as a
 * page's picture or a file's type; a closed window's own icon.
 */
export function windowIcon(item: TaskbarItem): Component {
  return tabIcon(item.tabs.find((tab) => tab.id === item.current) ?? item.tabs[0], item.icon)
}
