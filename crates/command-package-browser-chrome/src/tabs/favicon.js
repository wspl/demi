// The address of the page's icon, or null: the last icon the page names, else
// its site's /favicon.ico. It runs in a world of its own and only reads the
// document, so the page's scripts neither see nor change it, and it loads
// nothing: the Host loads and draws the icon (favicon.rs).
(() => {
  if (location.protocol !== 'http:' && location.protocol !== 'https:') {
    return null;
  }
  const named = [...document.querySelectorAll('link[rel][href]')].filter((link) => link.relList.contains('icon'));
  return named.length > 0 ? named[named.length - 1].href : new URL('/favicon.ico', location.href).href;
})()
