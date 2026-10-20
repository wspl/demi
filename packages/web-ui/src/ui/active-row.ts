/**
 * The row a listbox-style list's keys act on, and the one that wears the
 * focus ring while the list has focus (WAI-ARIA's listbox pattern): the first
 * of `preferred` that the list still holds, else its first row, so a list
 * that takes focus never leaves the user without a place. Rows are named by
 * a key (an id, a path); `null` when the list holds none.
 */
export function activeRow<K>(rows: readonly K[], ...preferred: readonly (K | null | undefined)[]): K | null {
  for (const key of preferred) {
    if (key !== null && key !== undefined && rows.includes(key))
      return key
  }
  return rows[0] ?? null
}

/**
 * Whether a list took focus from the keyboard (Tab), which is when it shows
 * its active row's ring; a click focuses it too and leaves none.
 */
export function focusedByKeyboard(event: FocusEvent): boolean {
  return event.currentTarget instanceof Element && event.currentTarget.matches(':focus-visible')
}
