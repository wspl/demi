import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { expect, test } from 'bun:test'

// The settings rail shows the open section's row when it opens, then stays
// where the user scrolls it (`SettingsDialog.vue`): the product lists the
// sections again whenever its state changes, such as a device coming online,
// and that once pulled the list back to the open row. A happy-dom page; a
// few ms besides its start.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick, ref } = await import('vue')
const { createOverlayStore } = await import('../../overlay/overlayStore')
const { settingsSections } = await import('../sections')
const { default: SettingsDialog } = await import('../SettingsDialog.vue')

test('the rail shows the open row once, and listing the sections again leaves the list where the user scrolled it', async () => {
  const revealed: string[] = []
  HTMLElement.prototype.scrollIntoView = function () {
    revealed.push(this.getAttribute('data-section') ?? '')
  }
  const sections = ref(settingsSections())
  const tab = ref<string | null>('models')
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () => h(SettingsDialog, {
      isOpen: true,
      overlayStore: createOverlayStore(),
      sections: sections.value,
      tab: tab.value,
      'onUpdate:tab': (value: string | null) => { tab.value = value },
    }),
  })
  app.mount(root)
  await nextTick()
  await nextTick()
  expect(revealed).toEqual(['models'])

  sections.value = settingsSections()
  await nextTick()
  await nextTick()
  expect(revealed).toEqual(['models'])

  tab.value = 'credentials'
  await nextTick()
  await nextTick()
  expect(revealed).toEqual(['models', 'credentials'])
  app.unmount()
})
