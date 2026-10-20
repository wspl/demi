import type { Component } from 'vue'
import { Copy, GitFork, RefreshCw, Share } from '@lucide/vue'
import type { ContextMenuEntry } from '../ui/context-menu'
import type { SentenceText, TitleText } from '../ui/ui-text'
import type { MessageForkState } from './message-fork'
import type { AnswerShare } from './sharing'
import { t } from './locales/text'
import { t as shared } from '../locales/text'

/** What an answer offers its actions from: what its footer is given. */
export interface AnswerActionsSource {
  fork?: () => Promise<void>
  forkState?: MessageForkState
  /** No Fork at all, as on a share's page, which acts on no conversation. */
  hideFork?: boolean
  share?: AnswerShare
  /** Share waits until the message completes, as Fork does. */
  shareWait?: boolean
  /**
   * Offered on the last answer exactly when editing the message before it is.
   */
  regenerate?: () => void
}

/**
 * One of an answer's actions, as its footer's button and its menu's row both
 * show it.
 */
export interface AnswerAction {
  key: 'copy' | 'fork' | 'share' | 'regenerate'
  icon: Component
  /** The button's name, on the answer itself. */
  label: TitleText
  /**
   * The menu row's name, which names its object beside a link's or a code
   * block's rows.
   */
  menuLabel: TitleText
  disabled: boolean
  disabledReason?: SentenceText
  /** Its work runs, as a fork being made. */
  loading: boolean
  run: () => unknown
}

/**
 * An answer's actions: the one owner of which there are, in which order and
 * when each is disabled. They are Copy, Fork, Share and Regenerate, each where
 * it is offered. The footer draws them as buttons, and the answer's menu as
 * rows. `copy` copies the answer.
 */
export function answerActions(source: AnswerActionsSource, copy: () => unknown): AnswerAction[] {
  const actions: AnswerAction[] = [
    { key: 'copy', icon: Copy, label: shared('common.copy'), menuLabel: t('transcript.copyMessage'), disabled: false, loading: false, run: copy },
  ]
  if (!source.hideFork) {
    const fork = source.fork
    actions.push({
      key: 'fork',
      icon: GitFork,
      label: t('transcript.assistant.fork'),
      menuLabel: t('transcript.assistant.fork'),
      disabled: !fork,
      disabledReason: t('transcript.assistant.forkWait'),
      loading: source.forkState?.phase === 'pending',
      run: () => fork?.(),
    })
  }
  const share = source.share
  if (share) {
    actions.push({
      key: 'share',
      icon: Share,
      label: t('transcript.assistant.share'),
      menuLabel: t('transcript.assistant.share'),
      disabled: source.shareWait === true,
      disabledReason: t('transcript.assistant.shareWait'),
      loading: false,
      run: () => share.start(),
    })
  }
  const regenerate = source.regenerate
  if (regenerate) {
    actions.push({
      key: 'regenerate',
      icon: RefreshCw,
      label: t('transcript.assistant.regenerate'),
      menuLabel: t('transcript.assistant.regenerate'),
      disabled: false,
      loading: false,
      run: regenerate,
    })
  }
  return actions
}

/**
 * The answer's actions as its menu's rows; a row whose work runs is not chosen
 * again.
 */
export function answerMenuEntries(actions: readonly AnswerAction[]): ContextMenuEntry[] {
  return actions.map((action) => ({
    icon: action.icon,
    label: action.menuLabel,
    disabled: action.disabled || action.loading,
    disabledReason: action.disabledReason,
    run: action.run,
  }))
}
