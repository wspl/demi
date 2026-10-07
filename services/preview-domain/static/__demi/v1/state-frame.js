// The preview's state frame, version 1 (docs/browser/preview.md § Page state): a hidden frame the
// Demi page opens on a preview origin to read that origin's storage, or to write a page state into
// it before the tab's page loads. It shares the storage of the origin's documents in the same Demi
// page, sessionStorage included, and answers on the port the Demi page sends with its request.
addEventListener('message', async event => {
  if (event.source !== parent || event.data?.type !== 'demi-preview-state' || !event.ports[0]) return;
  const [port] = event.ports;
  try {
    if (event.data.mode === 'dump') {
      port.postMessage({ storage: await __demiPageState.dump(event.data.origin) });
    } else {
      port.postMessage(await __demiPageState.seed(event.data.storage));
    }
  } catch (error) {
    port.postMessage({ error: String(error?.message ?? error) });
  }
});
