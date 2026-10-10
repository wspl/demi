<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * A reply streaming in, as the arrival chosen delivers it. The arrival
 * imitates a model: a few
 * characters at a time at its usual rate, a fast one, or bursts that arrive
 * after a pause, as a slow network delivers them.
 */
const ARRIVALS = {
  slow: { label: 'Slow', chunk: [3, 5], everyMs: [90, 140] },
  model: { label: 'Model', chunk: [4, 12], everyMs: [35, 80] },
  fast: { label: 'Fast', chunk: [30, 60], everyMs: [50, 70] },
  bursts: { label: 'Bursts', chunk: [300, 380], everyMs: [900, 1100] },
} as const
type Arrival = keyof typeof ARRIVALS
const arrivalOptions = (Object.keys(ARRIVALS) as Arrival[]).map((value) => ({ value, label: ARRIVALS[value].label }))
const languageOptions = [{ value: 'en', label: 'English' }, { value: 'zh', label: '中文' }] as const

const TEXTS = {
  en: `The login test fails because it still expects the old cookie name. The helper was renamed last week and now writes \`session\`, but the assertion in **auth.test.ts** looks for \`sid\`.

## What changed

The session cookie moved from \`sid\` to \`session\` when the auth middleware was split out of the router. Everything that writes the cookie went through the helper, so production is fine; only the test hard-codes the name.

- The helper in \`src/auth/cookie.ts\` writes the new name.
- The middleware reads it through the same helper.
- The test reads the raw header and compares it with a string literal.

## The fix

I changed the assertion to read the name from the helper, so the next rename cannot break the test again:

\`\`\`ts
expect(response.headers['set-cookie']).toContain(SESSION_COOKIE)
\`\`\`

All 42 auth tests pass now, including the expired-cookie case you asked about. If you want, I can also add a lint rule that flags string literals matching a cookie name, which would have caught this before the test ran.`,
  zh: `登录测试失败，是因为它还在找旧的 cookie 名。上周把辅助函数改了名，现在写的是 \`session\`，但 **auth.test.ts** 里的断言还在找 \`sid\`。

## 改了什么

认证中间件从路由里拆出来的时候，会话 cookie 从 \`sid\` 改成了 \`session\`。所有写 cookie 的地方都经过辅助函数，所以线上没有问题，只有测试把名字写死了。

- \`src/auth/cookie.ts\` 里的辅助函数写的是新名字。
- 中间件通过同一个辅助函数读取它。
- 测试直接读原始响应头，再拿它和一个字符串字面量比较。

## 怎么修

我把断言改成从辅助函数读取名字，这样下次再改名，测试也不会因此失败：

\`\`\`ts
expect(response.headers['set-cookie']).toContain(SESSION_COOKIE)
\`\`\`

现在 42 个认证测试全部通过，包括你问到的 cookie 过期的情况。如果需要，我还可以加一条 lint 规则，标出和 cookie 名相同的字符串字面量，这样在跑测试之前就能发现这类问题。`,
} as const

const arrival = ref<Arrival>('model')
const language = ref<'en' | 'zh'>('en')
const content = ref('')
const streaming = ref(false)
const run = ref(0)
let timer: ReturnType<typeof setTimeout> | undefined

// The same pseudo-random arrival on every replay, so the two sides and two runs can be compared.
function seeded(seed: number): () => number {
  let state = seed
  return () => {
    state = (state * 1_103_515_245 + 12_345) % 2_147_483_648
    return state / 2_147_483_648
  }
}

function play(): void {
  clearTimeout(timer)
  const full = TEXTS[language.value]
  const { chunk, everyMs } = ARRIVALS[arrival.value]
  const random = seeded(7)
  const between = (range: readonly [number, number]) => Math.round(range[0] + random() * (range[1] - range[0]))
  content.value = ''
  streaming.value = true
  run.value += 1
  const next = () => {
    content.value = full.slice(0, content.value.length + between(chunk))
    if (content.value.length < full.length) {
      timer = setTimeout(next, between(everyMs))
      return
    }
    timer = setTimeout(() => {
      streaming.value = false
    }, 200)
  }
  timer = setTimeout(next, 300)
}
watch([arrival, language], play, { immediate: true })
onBeforeUnmount(() => clearTimeout(timer))
</script>

<template>
  <GallerySection
    title="Streaming Text"
    note="A reply streams in a step at a time, about 22 characters cut on a word, each fading in over 400 ms; the steps come faster as text waits, keeping the view about half a second behind what has arrived, so many steps fade at once and the text settles a few lines at a time, as Claude's own reply does, rather than looking typed. Text that arrives in a burst shows at the fastest pace, 22 characters every 25 ms, slowing as it catches up; what has not shown when the stream ends shows at once, fading as one step."
  >
    <div class="mb-3 flex flex-wrap items-center gap-3">
      <Segmented v-model="arrival" :options="arrivalOptions" size="sm" />
      <Segmented v-model="language" :options="languageOptions" size="sm" />
      <Button size="sm" @click="play">Replay</Button>
    </div>
    <GallerySpecimen variant="a reply arriving" wide>
      <div class="gallery-frame h-[36rem] overflow-y-auto bg-surface px-6 py-5">
        <StreamedMarkdown
          :key="run"
          class="text-conversation text-fg-body"
          :content="content"
          :streaming="streaming"
        />
      </div>
    </GallerySpecimen>
  </GallerySection>
</template>
