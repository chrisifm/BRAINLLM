// Progressive enhancement: prompts and commands remain selectable without JavaScript.
const pageStatus = document.getElementById('copy-status');
const copyMessages = {
  agent: 'Agent prompt copied. Paste it into your coding agent.',
  command: 'Command copied. Paste it into your terminal.',
  commands: 'Installation commands copied. Paste them into your terminal.',
  benchmark: 'Benchmark prompt copied. Paste it into your coding agent.',
};
for (const button of document.querySelectorAll('[data-copy]')) {
  button.hidden = false;
  const originalLabel = button.textContent;
  button.addEventListener('click', async () => {
    const target = document.getElementById(button.dataset.copy);
    const copyStatus = button.closest('.hero-cmd')?.querySelector('.hero-cmd-status') || pageStatus;
    if (!target) return;
    button.disabled = true;
    button.textContent = originalLabel;
    try {
      if (!navigator.clipboard?.writeText) throw new Error('Clipboard unavailable');
      await navigator.clipboard.writeText(target.textContent.trim());
      button.textContent = 'Copied';
      if (copyStatus) copyStatus.textContent = copyMessages[button.dataset.copyKind] || 'Text copied.';
    } catch {
      button.textContent = 'Select and copy';
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(target);
      selection?.removeAllRanges();
      selection?.addRange(range);
      target.focus({ preventScroll: true });
      if (copyStatus) copyStatus.textContent = 'Automatic copy is unavailable. The text is selected; copy it with your keyboard or device menu.';
    } finally {
      button.disabled = false;
    }
  });
}

// Local snapshot first, then one public refresh: no credentials, polling or persistence.
async function boundedJson(response) {
  if (!response.ok) throw new Error('Contributor list unavailable');
  if (response.status === 204) return [];
  if (!response.body) throw new Error('Response stream unavailable');
  const reader = response.body.getReader(), chunks = [];
  let bytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (bytes > 512 * 1024) { await reader.cancel(); throw new Error('Contributor response too large'); }
      chunks.push(value);
    }
  } finally { reader.releaseLock(); }
  const buffer = new Uint8Array(bytes);
  let offset = 0;
  for (const chunk of chunks) { buffer.set(chunk, offset); offset += chunk.byteLength; }
  return JSON.parse(new TextDecoder().decode(buffer));
}
async function loadContributors() {
  const list = document.getElementById('contributors-list');
  const status = document.getElementById('contributors-status');
  if (!list || !status) return;
  const controller = new AbortController();
  const abort = () => controller.abort();
  const timeout = setTimeout(abort, 5000);
  window.addEventListener('pagehide', abort, { once: true });
  list.setAttribute('aria-busy', 'true');
  status.textContent = 'Loading public contributor profiles…';
  let snapshotShown = false;
  const render = records => {
    const fragment = document.createDocumentFragment();
    const seen = new Set();
    for (const record of records) {
      if (!record || typeof record.login !== 'string' || !/^[a-z\d][a-z\d-]{0,38}(?:\[bot\])?$/i.test(record.login)) continue;
      if (seen.has(record.login.toLowerCase())) continue;
      let avatar;
      try { avatar = new URL(record.avatar_url); } catch { continue; }
      if (avatar.protocol !== 'https:' || avatar.hostname !== 'avatars.githubusercontent.com' || avatar.port || avatar.username || avatar.password) continue;
      if (record.type !== 'User' && record.type !== 'Bot') continue;
      seen.add(record.login.toLowerCase());
      const item = document.createElement('li');
      item.className = 'contributor';
      const link = document.createElement('a');
      link.href = `https://github.com/${encodeURIComponent(record.login)}`;
      const img = document.createElement('img');
      avatar.searchParams.set('s', '72');
      img.src = avatar.href;
      img.alt = '';
      img.width = 36;
      img.height = 36;
      img.loading = 'lazy';
      img.decoding = 'async';
      img.referrerPolicy = 'no-referrer';
      img.addEventListener('error', () => img.remove(), { once: true });
      const name = document.createElement('span');
      name.textContent = `@${record.login}`;
      link.append(img, name);
      item.append(link);
      fragment.append(item);
    }
    list.replaceChildren(fragment);
    return seen.size;
  };
  try {
    try {
      const snapshot = await boundedJson(await fetch('contributors.json', { signal: controller.signal, credentials: 'omit' }));
      if (snapshot.schemaVersion !== 1 || snapshot.repository !== 'chrisifm/BRAINLLM' || !Array.isArray(snapshot.contributors) || snapshot.contributors.length > 2000) throw new Error('Invalid snapshot');
      snapshotShown = render(snapshot.contributors.slice(0, 100).map(record => ({ ...record, avatar_url: record.avatarUrl }))) > 0;
      if (snapshotShown) status.textContent = 'Recorded contributors. Checking GitHub for updates…';
    } catch { /* A missing snapshot does not prevent the public refresh. */ }
    const records = await boundedJson(await fetch('https://api.github.com/repos/chrisifm/BRAINLLM/contributors?per_page=100', {
      signal: controller.signal, credentials: 'omit', referrerPolicy: 'no-referrer',
      headers: { Accept: 'application/vnd.github+json' },
    }));
    if (!Array.isArray(records) || records.length > 100) throw new Error('Invalid contributor response');
    const count = render(records);
    status.textContent = count ? 'Contributors recognized by GitHub. Thank you for improving BRAINLLM.' : 'No contributor profiles are available yet. Explore the repository and make your first contribution.';
  } catch {
    status.textContent = snapshotShown ? 'Showing recorded contributors; live updates are unavailable. See GitHub for the latest list.' : 'Live profiles are unavailable right now. Use the GitHub contributor link below.';
  } finally {
    clearTimeout(timeout);
    window.removeEventListener('pagehide', abort);
    list.setAttribute('aria-busy', 'false');
  }
}
loadContributors();
