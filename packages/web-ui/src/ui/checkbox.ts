export function nextCheckbox(checked: boolean, partial = false): { checked: boolean, partial: boolean } {
  if (partial) return { checked: true, partial: false }
  return { checked: !checked, partial: false }
}

export function checkboxMark(checked: boolean, partial = false): 'checked' | 'unchecked' | 'partial' {
  if (partial) return 'partial'
  return checked ? 'checked' : 'unchecked'
}
