import { expect, test } from 'bun:test'
import { createSSRApp, defineComponent, h, type Component } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { LAYER_Z, provideStacking, stackAbove } from '../stacking'

/** A floating surface: it shows the z-index it stands at, then what opens inside it. */
function surface(name: string, inner?: Component): Component {
  return defineComponent({
    setup() {
      const z = stackAbove()
      return () => h('div', [`${name}:${z()}`, inner ? h(inner) : null])
    },
  })
}

/** A dialog that stands `depth` above the first, as one stacked over another does. */
function dialog(depth: number, inner: Component): Component {
  return defineComponent({
    setup() {
      provideStacking(() => LAYER_Z + depth)
      return () => h(inner)
    },
  })
}

test('a menu stands above the layer it opens from, a dialog stacked over another included', async () => {
  // Over the page, as before stacking: a menu at 50, its submenu above it.
  const onPage = await renderToString(createSSRApp(surface('menu', surface('submenu'))))
  expect(onPage).toBe('<div>menu:50<div>submenu:51<!----></div></div>')
  // In a profile editor stacked over Settings (51): the menu at 52 shows over the editor.
  const inStacked = await renderToString(createSSRApp(dialog(1, surface('menu', surface('submenu')))))
  expect(inStacked).toBe('<div>menu:52<div>submenu:53<!----></div></div>')
})
