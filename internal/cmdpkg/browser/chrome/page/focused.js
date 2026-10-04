() => {
  let active = document.activeElement;
  while (active) {
    if (active.shadowRoot?.activeElement) {
      active = active.shadowRoot.activeElement;
    } else if ((active.tagName === 'IFRAME' || active.tagName === 'FRAME') && active.contentDocument?.activeElement) {
      active = active.contentDocument.activeElement;
    } else {
      return active;
    }
  }
  return document.documentElement;
}
