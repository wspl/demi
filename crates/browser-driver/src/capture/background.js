// The live view's capture extension (`live-view.md` § Capture). The
// service worker keeps the offscreen document that captures and encodes tabs,
// and does what only it may: map a CDP target to its tab and grant capture.

const OFFSCREEN = 'offscreen.html';

async function ensureOffscreen() {
  if (await chrome.offscreen.hasDocument()) return;
  try {
    await chrome.offscreen.createDocument({
      url: OFFSCREEN,
      reasons: ['USER_MEDIA'],
      justification: 'Capture the tabs the live view watches',
    });
  } catch (error) {
    // A concurrent start created it first.
    if (!(await chrome.offscreen.hasDocument())) throw error;
  }
}

async function tabOf(target) {
  const targets = await chrome.debugger.getTargets();
  const found = targets.find(candidate => candidate.id === target);
  if (found?.tabId === undefined) throw new Error(`no tab for target ${target}`);
  return found.tabId;
}

// A capture of the same tab that just stopped can still be closing; Chrome
// grants a new one once it has.
async function streamId(tabId) {
  const deadline = performance.now() + 5000;
  for (;;) {
    try {
      return await chrome.tabCapture.getMediaStreamId({ targetTabId: tabId });
    } catch (error) {
      if (performance.now() >= deadline) throw error;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  }
}

// Stopping a track returns before Chrome releases the tab's capture.
async function released(tabId) {
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    const tabs = await chrome.tabCapture.getCapturedTabs();
    if (!tabs.some(tab => tab.tabId === tabId && ['pending', 'active'].includes(tab.status))) return;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw new Error('tab capture did not stop');
}

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (sender.id !== chrome.runtime.id) return false;
  if (message.type === 'reset') {
    respond({});
    chrome.runtime.reload();
    return false;
  }
  const work = message.type === 'grant'
    ? tabOf(message.target).then(async tabId => ({ tabId, streamId: await streamId(tabId) }))
    : message.type === 'released'
      ? released(message.tabId).then(() => ({}))
      : null;
  if (!work) return false;
  work.then(respond, error => respond({ error: String(error?.message ?? error) }));
  return true;
});

ensureOffscreen().catch(error => console.error('Live view capture could not start', error));
