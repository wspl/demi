// The page's icon drawn 32 pixels square, as a PNG `data:` URL, or null: the
// last icon the page names, else its site's /favicon.ico. It runs in a world
// of its own, so the page's scripts neither see nor change it. An icon from
// another site that does not allow it taints the canvas, and the tab has none.
(async () => {
  if (location.protocol !== 'http:' && location.protocol !== 'https:') {
    return null;
  }
  const named = [...document.querySelectorAll('link[rel][href]')].filter((link) => link.relList.contains('icon'));
  const image = new Image();
  image.crossOrigin = 'anonymous';
  image.src = named.length > 0 ? named[named.length - 1].href : new URL('/favicon.ico', location.href).href;
  try {
    await image.decode();
  } catch {
    return null;
  }
  const canvas = document.createElement('canvas');
  canvas.width = 32;
  canvas.height = 32;
  const context = canvas.getContext('2d');
  if (!context) {
    return null;
  }
  context.drawImage(image, 0, 0, 32, 32);
  try {
    return canvas.toDataURL('image/png');
  } catch {
    return null;
  }
})()
