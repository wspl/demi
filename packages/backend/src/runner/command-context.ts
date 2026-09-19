import type {
  CommandCaller,
  CommandContext,
  CommandLocale
} from '@demicodes/command-protocol'
import type { ControlService } from '../storage/control'

/**
 * The locale commands receive until the conversation's user's browser reports
 * one (`native-runtime.md` § Command context).
 */
export const DEFAULT_LOCALE: CommandLocale = {
  timeZone: 'UTC',
  languages: ['en-US']
}

/**
 * What a job's or a user stream's declared commands receive. The backend is
 * its only source: it builds the context when the work starts, from the
 * conversation's user's preferences.
 */
export async function buildCommandContext(
  control: Pick<ControlService, 'getConversation' | 'getUserPreferences'>,
  conversation: string,
  caller: CommandCaller
): Promise<CommandContext> {
  const record = await control.getConversation(conversation)
  if (!record)
    throw new Error(`No conversation ${conversation}`)
  const { locale } = await control.getUserPreferences(record.userId)
  return { conversation, caller, locale: locale ?? DEFAULT_LOCALE }
}
