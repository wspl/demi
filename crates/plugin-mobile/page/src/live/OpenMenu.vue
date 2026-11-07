<script setup lang="ts">
import { computed, onMounted, shallowRef } from 'vue'
import { Menu, MenuGroup, MenuItem } from '@demicodes/plugin-sdk'
import type { ListResult, MobilePlatform, Simulator } from '../generated/plugin'
import { MobileError } from '../devices'
import type { MobileDevicesController } from './controller'
import { t } from '../locales/text'

/**
 * The simulators of the conversation's device as the taskbar's Open lists
 * them, as Simulator.app's File > Open Simulator does: grouped by system,
 * newest first, then each platform the device cannot run. Choosing one
 * opens it (`MobileDevicesController.open`) and its window.
 *
 * The list is read each time the menu shows, as a device's simulators come
 * and go outside Demi.
 */
const props = defineProps<{ controller: MobileDevicesController }>()
const emit = defineEmits<{ opened: [] }>()

const listed = shallowRef<ListResult | null>(null)
const failure = shallowRef<string | null>(null)
onMounted(async () => {
  try {
    listed.value = await props.controller.api.list()
  } catch (error) {
    failure.value = MobileError.of(error).message
  }
})

const PLATFORMS: readonly MobilePlatform[] = ['ios', 'android']

/** A system's version as numbers, which order it: iOS 26.5 is [26, 5]. */
function version(os: string): number[] {
  return (os.match(/\d+/g) ?? []).map(Number)
}

function newer(a: string, b: string): number {
  const [x, y] = [version(a), version(b)]
  for (let at = 0; at < Math.max(x.length, y.length); at += 1) {
    const by = (y[at] ?? 0) - (x[at] ?? 0)
    if (by !== 0) {
      return by
    }
  }
  return 0
}

const groups = computed(() => {
  const devices = listed.value?.devices ?? []
  const systems = [...new Set(devices.map((each) => `${each.platform}\n${each.os}`))]
    .map((key) => devices.find((each) => `${each.platform}\n${each.os}` === key)!)
    .sort((a, b) => PLATFORMS.indexOf(a.platform) - PLATFORMS.indexOf(b.platform) || newer(a.os, b.os))
  return systems.map((system) => ({
    os: system.os,
    simulators: devices.filter((each) => each.platform === system.platform && each.os === system.os),
  }))
})

function state(simulator: Simulator): string | undefined {
  if (simulator.open !== undefined) {
    return t('open.here')
  }
  return simulator.state === 'booted' ? t('open.running') : undefined
}

function choose(simulator: Simulator): void {
  void props.controller.open(simulator)
  emit('opened')
}
</script>

<template>
  <!-- One width, loading or listed, so the submenu does not change its size as the list comes. -->
  <Menu class="w-72">
    <MenuItem v-if="!listed && !failure" loading :label="t('open.loading')" />
    <MenuItem v-else-if="failure" :label="t('open.listFailed')" disabled :disabled-reason="failure" />
    <template v-else-if="listed">
      <MenuItem v-if="listed.devices.length === 0 && listed.unavailable.length === 0" :label="t('open.none')" disabled />
      <MenuGroup v-for="group in groups" :key="group.os" :label="group.os">
        <MenuItem
          v-for="simulator in group.simulators"
          :key="simulator.nativeId"
          :label="simulator.name"
          :detail="simulator.model !== simulator.name ? simulator.model : undefined"
          :value="state(simulator)"
          :disabled="simulator.elsewhere"
          :disabled-reason="t('open.elsewhere')"
          @select="choose(simulator)"
        />
      </MenuGroup>
      <MenuGroup v-for="each in listed.unavailable" :key="each.platform" :label="t(`platforms.${each.platform}`)">
        <MenuItem :label="t('open.unavailable')" disabled :disabled-reason="each.reason" />
      </MenuGroup>
    </template>
  </Menu>
</template>
