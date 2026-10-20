import { t } from '../locales/text'

/**
 * The title a conversation shows: its own, or for the placeholder, which
 * holds no words (null), New conversation in the reader's language.
 */
export function conversationTitle(title: string | null): string {
  return title ?? t('conversation.untitled')
}
