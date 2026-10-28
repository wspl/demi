<script setup lang="ts">
// A notification as iOS shows one at the top of the screen: the app's icon,
// its name, the title and the text. The demo draws it, as the phone's system
// would over the app.
import appIcon from '@demicodes/web-ui/assets/app-icon.png'
import type { Notice } from './scenes'

defineProps<{ notice: Notice | null }>()
</script>

<template>
  <Transition name="notice">
    <div v-if="notice" class="notice" role="status">
      <img class="app-icon" :src="appIcon" alt="" width="38" height="38" />
      <div class="body">
        <p class="top"><b>{{ notice.title }}</b><span>now</span></p>
        <p class="text">{{ notice.text }}</p>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.notice {
  position: absolute;
  top: 56px;
  right: 10px;
  left: 10px;
  z-index: 200;
  display: flex;
  gap: 10px;
  align-items: center;
  padding: 12px 14px;
  border-radius: 22px;
  background: rgb(246 242 238 / 82%);
  box-shadow: 0 10px 30px rgb(40 25 15 / 18%);
  color: #1d1a18;
  font: 15px/1.3 -apple-system, BlinkMacSystemFont, 'Inter Variable', sans-serif;
  backdrop-filter: blur(20px) saturate(1.6);
}

.app-icon {
  flex: none;
  width: 38px;
  height: 38px;
  border-radius: 9px;
}

.body {
  min-width: 0;
  flex: 1;
}

.top {
  display: flex;
  justify-content: space-between;
  gap: 8px;
}

.top span {
  color: #77706a;
  font-size: 13px;
}

.text {
  display: -webkit-box;
  overflow: hidden;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}

.notice-enter-active,
.notice-leave-active {
  transition: transform 0.5s cubic-bezier(0.2, 0.9, 0.3, 1.15), opacity 0.4s ease;
}

.notice-enter-from,
.notice-leave-to {
  opacity: 0;
  transform: translateY(-120%);
}
</style>
