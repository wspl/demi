async function(values, labels, indices, apply) {
  if ((await elementState.call(this, ['enabled'], false)).failed) return {status: 'disabled', values: []};
  const requests = values || labels || indices;
  const options = Array.from(this.options);
  const selected = [];
  const matched = new Set();
  for (const [index, option] of options.entries()) {
    const candidate = values ? option.value : labels ? option.label : index;
    if (!requests.includes(candidate) || matched.has(candidate)) continue;
    if ((await elementState.call(option, ['enabled'], false)).failed) {
      return {status: 'disabled', values: []};
    }
    selected.push(option);
    matched.add(candidate);
    if (!this.multiple) break;
  }
  if (!selected.length || (this.multiple && requests.some(request => !matched.has(request)))) {
    return {status: 'missing', values: []};
  }
  if (apply) {
    for (const option of options) option.selected = selected.includes(option);
    this.dispatchEvent(new Event('input', {bubbles: true, composed: true}));
    this.dispatchEvent(new Event('change', {bubbles: true}));
  }
  return {status: 'ready', values: Array.from(apply ? this.selectedOptions : selected, option => option.value)};
}
