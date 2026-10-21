import { computed, shallowRef, watch, type ComputedRef } from 'vue'
import { hasAnswered } from './file-cache'
import { useShowings } from './showing'
import type { CommitHistorySource, CommitLogPage, LoggedCommit } from './history'
import { t } from './locales/text'

/** One branch's commits as the list shows them, page after page. */
export interface CommitLogView {
  readonly commits: ComputedRef<readonly LoggedCommit[]>
  /** Older commits follow the ones listed. */
  readonly more: ComputedRef<boolean>
  /** A page is on its way that nothing shows yet. */
  readonly loading: ComputedRef<boolean>
  /** The newest page has an answer, or the reason there is none. */
  readonly listed: ComputedRef<boolean>
  /** The branch is gone: deleted since it was shown. */
  readonly gone: ComputedRef<boolean>
  /** Why a page could not be read, the newest or a later one, which `loadMore` reads again. */
  readonly failure: ComputedRef<string | null>
  /**
   * Reads a page that could not be read again, else shows the next page,
   * while older commits follow and the last one shown has come.
   */
  loadMore(): void
}

/**
 * The commits of `branch` (HEAD's for null) from `source`, while the calling
 * component lives: the newest page first, and each further page the list asks
 * for, each a kept entry the host keeps current.
 *
 * A page names the commit it continues after. So when a commit or a rewrite
 * changes the end of one page, the pages after it no longer follow and are let
 * go; the list asks for them again as it scrolls.
 */
export function useCommitLog(
  source: () => CommitHistorySource | null,
  branch: () => string | null,
): CommitLogView {
  /** What each page continues after: '' for the newest, then the last commit of the page before. */
  const pages = shallowRef<readonly string[]>([''])
  const owner = computed(() => {
    const history = source()
    return history ? { history, branch: branch() } : null
  })
  watch(owner, () => {
    pages.value = ['']
  })
  const shown = useShowings(() => owner.value, () => pages.value, ({ history, branch }, after) => history.showLog(branch, after))

  /** The pages that follow on from each other, in order, up to the first not read yet. */
  const chain = computed(() => {
    const read: CommitLogPage[] = []
    for (const [index, after] of pages.value.entries()) {
      const page = shown.entries.value.get(after)?.value
      if (!page)
        break
      const before = read.at(-1)
      if (index > 0 && before?.commits.at(-1)?.id !== after)
        break
      read.push(page)
    }
    return read
  })
  // A page that no longer follows the one before it is let go.
  watch(chain, (read) => {
    const next = pages.value[read.length]
    if (next !== undefined && shown.entries.value.get(next)?.value)
      pages.value = pages.value.slice(0, read.length)
  })
  const commits = computed(() => chain.value.flatMap((page) => page.commits))
  const more = computed(() => chain.value.at(-1)?.more ?? false)
  const first = computed(() => shown.entries.value.get(''))
  /** The first page asked for that could not be read and has nothing to show. */
  const failed = computed(() => pages.value.find((after) => {
    const entry = shown.entries.value.get(after)
    return entry?.failure && entry.answer === null && entry.value === undefined
  }))
  return {
    commits,
    more,
    listed: computed(() => hasAnswered(first.value)),
    loading: computed(() => [...shown.entries.value.values()].some((entry) => entry.value === undefined && entry.failure === null && entry.answer === null)),
    gone: computed(() => first.value?.answer === 'gone'),
    failure: computed(() => {
      const after = failed.value
      if (after === undefined)
        return null
      return shown.entries.value.get(after)?.failure?.message ?? t('changes.commitsFailed')
    }),
    loadMore() {
      if (failed.value !== undefined) {
        void shown.retry(failed.value)
        return
      }
      const read = chain.value
      if (read.length !== pages.value.length || !more.value)
        return
      const last = read.at(-1)?.commits.at(-1)?.id
      if (last !== undefined)
        pages.value = [...pages.value, last]
    },
  }
}
