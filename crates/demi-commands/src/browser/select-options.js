function(values, labels, indices) {
  if (!(this instanceof HTMLSelectElement)) {
    throw new Error('select target is not a select control');
  }
  const requests = [values, labels, indices].filter(value => value !== null);
  if (requests.length !== 1 || requests[0].length === 0) {
    throw new Error('provide exactly one nonempty option value, label, or index list');
  }
  const options = Array.from(this.options);
  const selected = requests[0].map(request => {
    const matches = options.filter((option, index) => values !== null
      ? option.value === request
      : labels !== null ? option.label === request : index === request);
    if (matches.length !== 1 || matches[0].disabled || matches[0].parentElement.disabled) {
      throw new Error('option is missing, ambiguous, or disabled');
    }
    return matches[0];
  });
  if (!this.multiple && selected.length !== 1) {
    throw new Error('single-select control requires exactly one option');
  }
  for (const option of options) {
    option.selected = selected.includes(option);
  }
  this.dispatchEvent(new Event('input', { bubbles: true }));
  this.dispatchEvent(new Event('change', { bubbles: true }));
  return selected.map(option => option.value);
}
