import type { InjectionKey, Ref } from 'vue'

/**
 * Which of the taskbar's buttons shows its menu, by the button's id, as one
 * the taskbar draws outside it, such as Schedules, learns it: a menu that
 * resting on a button opens closes another's at once, so two never show.
 */
export const taskbarMenuKey: InjectionKey<Ref<string | null>> = Symbol('taskbarMenu')
