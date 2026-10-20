import { readonly, shallowRef } from 'vue'
import { appThemeStore } from '../theme/appTheme'

// When rendered Markdown should render again: a chunk it waits for has
// arrived, the highlighter, a language or KaTeX, or the page's mode has
// changed, which code shows in.

const renderVersion = shallowRef(0)

// The subscription lasts as long as the page, like the loaded chunks.
appThemeStore.subscribe(renderAgain)

/** Has what shows rendered Markdown render it again. */
export function renderAgain(): void {
  renderVersion.value += 1
}

/** Changes when rendered Markdown should render again. */
export function useMarkdownRenderVersion() {
  return readonly(renderVersion)
}
