import type { AppContext, Component } from 'vue'
import { h, markRaw, render } from 'vue'

interface VueRendererOptions {
  appContext?: AppContext | null
  useContainer?: boolean
}

export class VueRenderer {
  private el: HTMLElement
  private component: Component
  private props: Record<string, unknown>
  private useContainer: boolean
  private appContext: AppContext | null
  private destroyed = false

  constructor(component: Component, props: Record<string, unknown> = {}, options: VueRendererOptions = {}) {
    this.el = document.createElement('div')
    this.component = markRaw(component)
    this.props = { ...props }
    this.useContainer = options.useContainer ?? false
    this.appContext = options.appContext ?? null
    this.patch()
  }

  get dom(): HTMLElement {
    if (this.useContainer) return this.el
    return this.el.firstElementChild as HTMLElement ?? this.el
  }

  updateProps(props: Record<string, unknown>) {
    if (this.destroyed) return
    for (const [key, value] of Object.entries(props)) {
      this.props[key] = value
    }
    this.patch()
  }

  private patch() {
    const vnode = h(this.component, { ...this.props })
    if (this.appContext) vnode.appContext = this.appContext
    render(vnode, this.el)
  }

  destroy() {
    if (this.destroyed) return
    this.destroyed = true
    render(null, this.el)
  }
}
