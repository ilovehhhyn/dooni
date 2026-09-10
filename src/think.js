/* Rich captures use a small, safe document vocabulary, never stored arbitrary HTML. */
const thinkInput = document.getElementById('think-input');
let thinkData = { items: [], draft: null };
let thinkReady = false;
const expandedThoughts = new Set();
let savingThought = false;
let saveQueue = Promise.resolve();
const invokeThink = (command, args) => window.__TAURI__.core.invoke(command, args);
const webURL = value => { try { const url = new URL(value); return ['https:', 'http:'].includes(url.protocol) ? url.href : null; } catch { return null; } };
function readBlocks(root) {
  const blocks = [];
  function walk(node) {
    if (node.nodeType === Node.TEXT_NODE) { if (node.textContent) blocks.push({ type: 'text', text: node.textContent }); return; }
    if (node.nodeName === 'IMG') { if (/^data:image\/(png|jpeg|webp|gif);base64,/.test(node.src)) blocks.push({ type: 'image', src: node.src }); return; }
    if (node.nodeName === 'A' && webURL(node.href)) { blocks.push({ type: 'link', text: node.textContent || new URL(node.href).hostname, href: node.href }); return; }
    if (node.nodeName === 'BR') { blocks.push({ type: 'text', text: '\n' }); return; }
    if (['SCRIPT', 'STYLE'].includes(node.nodeName)) return;
    if (['DIV', 'P', 'LI'].includes(node.nodeName) && blocks.length) blocks.push({ type: 'text', text: '\n' });
    node.childNodes.forEach(walk);
  }
  root.childNodes.forEach(walk);
  return blocks;
}
function renderBlocks(root, blocks) {
  root.replaceChildren();
  for (const block of blocks || []) {
    if (block.type === 'image' && /^data:image\/(png|jpeg|webp|gif);base64,/.test(block.src)) {
      const image = document.createElement('img'); image.src = block.src; image.alt = 'Saved image'; root.append(image);
    } else if (block.type === 'link' && webURL(block.href)) {
      const link = document.createElement('a'); link.href = block.href; link.textContent = block.text; link.title = block.href; root.append(link);
    } else if (block.type === 'text') {
      // Bare URLs become readable links; named links keep their original label.
      for (const part of block.text.split(/(https?:\/\/[^\s<>]+)/g)) {
        if (webURL(part)) { const link = document.createElement('a'); link.href = part; const url = new URL(part); link.textContent = url.hostname.replace(/^www\./, '') + (url.pathname === '/' ? '' : url.pathname); link.title = part; root.append(link); }
        else root.append(document.createTextNode(part));
      }
    }
  }
}
function thoughtText(blocks) { return blocks.map(b => b.type === 'link' ? `${b.text} (${b.href})` : b.type === 'text' ? b.text : '').join('').trim(); }
function thoughtTitle(blocks) { return blocks.map(b => b.type === 'image' ? '' : b.text).join('').trim() || 'An image to think about'; }
function persistThink() {
  const data = JSON.parse(JSON.stringify(thinkData));
  saveQueue = saveQueue.catch(() => {}).then(() => invokeThink('save_think_data', { data }));
  return saveQueue;
}
function saveDraft() {
  if (!thinkReady || savingThought) return;
  thinkData.draft = { blocks: readBlocks(thinkInput) };
  persistThink().catch(error => setStatus(`Couldn't save draft: ${error}`));
}
function switchHome(tab) {
  for (const name of ['think', 'chats']) {
    const active = name === tab;
    document.getElementById(`home-${name}`).classList.toggle('hidden', !active);
    const button = document.getElementById(`tab-${name}`); button.setAttribute('aria-selected', String(active)); button.tabIndex = active ? 0 : -1;
  }
}
for (const name of ['think', 'chats']) {
  const button = document.getElementById(`tab-${name}`);
  button.addEventListener('click', () => switchHome(name));
  button.addEventListener('keydown', event => { if (['ArrowLeft', 'ArrowRight'].includes(event.key)) { event.preventDefault(); const other = name === 'think' ? 'chats' : 'think'; switchHome(other); document.getElementById(`tab-${other}`).focus(); } });
}
function action(label, callback) { const button = document.createElement('button'); button.type = 'button'; button.className = 'quiet-button'; button.textContent = label; button.addEventListener('click', callback); return button; }
function renderThoughts() {
  const list = document.getElementById('think-list'); list.replaceChildren();
  const items = thinkData.items.filter(item => !item.deletedAt)
    .sort((a, b) => Number(Boolean(b.pinned)) - Number(Boolean(a.pinned)));
  if (!items.length) {
    const empty = document.createElement('li'); empty.className = 'think-empty';
    empty.textContent = 'a big space for your big mind';
    list.append(empty);
  }
  const shownDates = new Set();
  for (const item of items) {
    const card = document.createElement('li'); card.className = 'thought-card'; card.dataset.thoughtId = item.id;
    const body = document.createElement('div'); body.className = 'thought-content';
    const editing = Boolean(thinkData.editDrafts?.[item.id]);
    renderBlocks(body, editing ? thinkData.editDrafts[item.id] : item.blocks);
    const expanded = expandedThoughts.has(item.id);
    card.classList.toggle('expanded', expanded);
    card.classList.toggle('is-editing', editing);
    const remove = action('', async () => {
      remove.disabled = true; item.deletedAt = Date.now();
      try {
        await persistThink(); renderThoughts();
        setStatus('Thought deleted');
        const undo = action('Undo', async () => {
          const deletedAt = item.deletedAt; delete item.deletedAt; undo.disabled = true;
          try { await persistThink(); renderThoughts(); setStatus('Thought restored'); }
          catch (error) { item.deletedAt = deletedAt; undo.disabled = false; setStatus(`Couldn't restore thought: ${error}`); }
        });
        els.status.append(undo);
      } catch (error) { delete item.deletedAt; remove.disabled = false; setStatus(`Couldn't delete thought: ${error}`); }
    });
    remove.classList.add('thought-delete'); remove.title = 'Delete thought'; remove.setAttribute('aria-label', 'Delete thought');
    remove.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 4.5h9M6 2.5h4l.5 2H5.5l.5-2ZM5 6.5l.5 6h5l.5-6M7 7v4M9 7v4"/></svg>';
    const pin = action('', async () => {
      const previous = Boolean(item.pinned); item.pinned = !previous; pin.disabled = true;
      try {
        await persistThink(); renderThoughts();
        document.querySelector(`[data-thought-id="${item.id}"] .thought-pin`)?.focus();
      } catch (error) { item.pinned = previous; pin.disabled = false; setStatus(`Couldn't save pin: ${error}`); }
    });
    pin.classList.add('thought-pin'); pin.disabled = editing;
    pin.title = item.pinned ? 'Unpin thought' : 'Pin thought';
    pin.setAttribute('aria-label', pin.title); pin.setAttribute('aria-pressed', String(Boolean(item.pinned)));
    pin.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5 2h6l-1 4 2 3H4l2-3-1-4Z"/><path d="M8 9v5"/></svg>';
    card.classList.toggle('is-pinned', Boolean(item.pinned));
    const footer = document.createElement('div'); footer.className = 'thought-footer';
    const actions = document.createElement('div'); actions.className = 'thought-actions';
    for (const provider of ['Codex', 'Claude']) actions.append(action(`Ask ${provider} ↗`, event => startThought(item, provider, event.currentTarget)));
    const edit = action('Edit', () => {
      thinkData.editDrafts ||= {}; thinkData.editDrafts[item.id] = structuredClone(item.blocks);
      renderThoughts(); document.querySelector(`[data-thought-id="${item.id}"] .thought-content`)?.focus();
      persistThink().catch(error => setStatus(String(error)));
    });
    edit.classList.add('thought-edit'); edit.title = 'Edit thought'; edit.setAttribute('aria-label', 'Edit thought');
    edit.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="m10.8 2.5 2.7 2.7-7.8 7.8-3.4.7.7-3.4 7.8-7.8ZM9.3 4l2.7 2.7"></path></svg>';
    const expand = action(expanded ? '⌃' : '⌄', () => {
      if (expandedThoughts.has(item.id)) expandedThoughts.delete(item.id); else expandedThoughts.add(item.id);
      renderThoughts(); document.querySelector(`[data-thought-id="${item.id}"] .thought-expand`)?.focus();
    });
    expand.classList.add('thought-expand'); expand.setAttribute('aria-expanded', String(expanded));
    expand.setAttribute('aria-label', expanded ? 'Collapse thought' : 'Expand thought');
    const updateDisclosure = () => { const truncated = body.clientHeight >= 131 && body.scrollHeight > body.clientHeight + 2; expand.hidden = editing || (!expanded && !truncated); card.classList.toggle("is-truncated", !editing && !expanded && truncated); };
    body.querySelectorAll('img').forEach(img => img.addEventListener('load', updateDisclosure));
    if (editing) {
      body.contentEditable = 'true'; body.setAttribute('role', 'textbox'); body.setAttribute('aria-label', 'Edit thought'); body.setAttribute('aria-multiline', 'true');
      const saveEditDraft = () => {
        thinkData.editDrafts[item.id] = readBlocks(body);
        persistThink().catch(error => setStatus(`Couldn't save edit: ${error}`));
      };
      body.addEventListener('input', saveEditDraft);
      body.addEventListener('paste', event => pasteIntoEditor(event, body, saveEditDraft));
      body.addEventListener('dragover', event => event.preventDefault());
      body.addEventListener('drop', event => {
        event.preventDefault(); insertImages([...event.dataTransfer.files], body, saveEditDraft).catch(error => setStatus(String(error)));
      });
      body.addEventListener('keydown', event => {
        if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); openLinkEditor(body, saveEditDraft); }
        if (event.key === 'Escape') { event.preventDefault(); cancel.click(); }
      });
      const save = action('Save', async () => {
        const blocks = readBlocks(body); if (!blocks.some(b => b.type !== 'text' || b.text.trim())) { setStatus('Keep some text or an image, or cancel this edit.'); return; }
        save.disabled = true; body.contentEditable = 'false';
        const previous = item.blocks;
        item.blocks = blocks; delete thinkData.editDrafts[item.id];
        try { await persistThink(); renderThoughts(); }
        catch (error) { item.blocks = previous; thinkData.editDrafts[item.id] = blocks; body.contentEditable = 'true'; save.disabled = false; setStatus(String(error)); }
      });
      const cancel = action('Cancel', async () => {
        const draft = thinkData.editDrafts[item.id]; delete thinkData.editDrafts[item.id];
        try { await persistThink(); renderThoughts(); }
        catch (error) { thinkData.editDrafts[item.id] = draft; setStatus(String(error)); }
      });
      footer.append(cancel, save);
    } else {
      footer.append(actions);
      body.tabIndex = 0; body.setAttribute('role', 'button'); body.setAttribute('aria-label', 'Copy thought');
      let copying = false;
      const copy = async () => {
        if (copying) return; copying = true; card.setAttribute('aria-busy', 'true');
        try {
          await invokeThink('copy_think_content', { text: thoughtText(item.blocks), images: item.blocks.filter(b => b.type === 'image').map(b => b.src) });
          setStatus('Copied');
        } catch (error) { setStatus(`Couldn't copy thought: ${error}`); }
        finally { copying = false; card.removeAttribute('aria-busy'); }
      };
      card.addEventListener('click', event => {
        if (event.target.closest('button, a, [contenteditable="true"]') || window.getSelection()?.toString()) return;
        copy();
      });
      body.addEventListener('keydown', event => {
        if (event.target !== body || !['Enter', ' '].includes(event.key)) return;
        event.preventDefault(); copy();
      });
      card.append(edit);
    }
    const date = document.createElement('time'); date.className = 'thought-date';
    if (item.createdAt && !Number.isNaN(new Date(item.createdAt).getTime())) {
      date.dateTime = new Date(item.createdAt).toISOString();
      date.textContent = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(new Date(item.createdAt));
      date.title = new Date(item.createdAt).toLocaleString();
    }
    const dateKey = item.createdAt ? new Date(item.createdAt).toLocaleDateString() : '';
    if (dateKey && !shownDates.has(dateKey)) {
      shownDates.add(dateKey); card.append(date); card.classList.add('has-date');
    }
    card.append(remove, pin, body, footer, expand); list.append(card);
    updateDisclosure();
    requestAnimationFrame(updateDisclosure);
  }
}
async function startThought(item, provider, button) {
  const buttons = button.closest('.thought-actions').querySelectorAll('button'); buttons.forEach(b => b.disabled = true);
  setStatus('Preparing your thought…');
  try {
    await invokeThink('copy_think_content', { text: thoughtText(item.blocks), images: item.blocks.filter(b => b.type === 'image').map(b => b.src) });
    const prefilled = await invokeThink('open_think_app', { provider, prompt: item.blocks.some(b => b.type === 'image') ? null : thoughtText(item.blocks) });
    setStatus(prefilled ? `Opened in ${provider}, ready to send.` : `Opened ${provider}. Press ⌘V to paste your thought.`);
  } catch (error) { setStatus(`Couldn't open thought: ${error}`); }
  finally { buttons.forEach(b => b.disabled = false); }
}
async function insertImages(files, editor = thinkInput, onChange = saveDraft) {
  for (const file of files) {
    if (!/^image\/(png|jpeg|webp|gif)$/.test(file.type)) { setStatus('Use a PNG, JPEG, WebP, or GIF image.'); continue; }
    if (file.size > 12 * 1024 * 1024) { setStatus('Please use an image smaller than 12 MB.'); continue; }
    const src = await new Promise((resolve, reject) => { const reader = new FileReader(); reader.onload = () => resolve(reader.result); reader.onerror = reject; reader.readAsDataURL(file); });
    const img = document.createElement('img'); img.src = src; img.alt = 'Saved image'; editor.append(img); editor.append(document.createElement('br'));
  }
  editor.focus(); onChange();
}
function pasteIntoEditor(event, editor, onChange) {
  event.preventDefault();
  const images = [...event.clipboardData.files].filter(file => file.type.startsWith('image/'));
  if (images.length) { insertImages(images, editor, onChange).catch(error => setStatus(String(error))); return; }
  const html = event.clipboardData.getData('text/html');
  const holder = document.createElement('div');
  if (html) { const parsed = new DOMParser().parseFromString(html, 'text/html'); renderBlocks(holder, readBlocks(parsed.body)); }
  else renderBlocks(holder, [{ type: 'text', text: event.clipboardData.getData('text/plain') }]);
  document.execCommand('insertHTML', false, holder.innerHTML); onChange();
}
thinkInput.addEventListener('paste', event => pasteIntoEditor(event, thinkInput, saveDraft));
thinkInput.addEventListener('input', saveDraft);
thinkInput.addEventListener('keydown', event => {
  if (event.isComposing) return;
  if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); document.getElementById('think-form').requestSubmit(); }
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); openLinkEditor(); }
});
thinkInput.addEventListener('dragover', event => event.preventDefault());
thinkInput.addEventListener('drop', event => { event.preventDefault(); insertImages([...event.dataTransfer.files]).catch(error => setStatus(String(error))); });
document.getElementById('attach-image').addEventListener('click', () => document.getElementById('image-picker').click());
document.getElementById('image-picker').addEventListener('change', event => { insertImages([...event.target.files]).catch(error => setStatus(String(error))); event.target.value = ''; });
let linkSelection = null;
let linkTarget = thinkInput;
let linkChanged = saveDraft;
function openLinkEditor(editor = thinkInput, onChange = saveDraft) {
  linkTarget = editor; linkChanged = onChange;
  const selection = window.getSelection(); linkSelection = selection.rangeCount && editor.contains(selection.anchorNode) ? selection.getRangeAt(0).cloneRange() : null;
  document.getElementById('link-label').value = linkSelection?.toString() || ''; document.getElementById('link-url').value = '';
  document.getElementById('link-modal').classList.remove('hidden'); document.getElementById('link-url').focus();
}
function closeLinkEditor() { document.getElementById('link-modal').classList.add('hidden'); linkTarget.focus(); }
document.getElementById('attach-link').addEventListener('click', () => openLinkEditor());
document.getElementById('link-close').addEventListener('click', closeLinkEditor);
document.getElementById('link-modal').addEventListener('keydown', event => { if (event.key === 'Escape') closeLinkEditor(); });
document.getElementById('link-form').addEventListener('submit', event => {
  event.preventDefault(); const href = webURL(document.getElementById('link-url').value.trim()); if (!href) { setStatus('Add a link starting with https://'); return; }
  const link = document.createElement('a'); link.href = href; link.textContent = document.getElementById('link-label').value.trim() || new URL(href).hostname;
  closeLinkEditor();
  if (linkSelection) { linkSelection.deleteContents(); linkSelection.insertNode(link); } else linkTarget.append(link);
  linkChanged();
});
document.addEventListener('click', event => {
  const link = event.target.closest('.thought-content a, #think-input a');
  if (!link) return;
  if (link.closest('[contenteditable="true"]') && !event.metaKey) { event.preventDefault(); return; }
  event.preventDefault(); invokeThink('open_think_link', { url: link.href }).catch(error => setStatus(String(error)));
});
document.getElementById('think-form').addEventListener('submit', async event => {
  event.preventDefault(); if (!thinkReady || savingThought) return;
  const blocks = readBlocks(thinkInput); if (!blocks.some(b => b.type !== 'text' || b.text.trim())) return;
  savingThought = true; thinkInput.contentEditable = 'false';
  const previous = structuredClone(thinkData);
  try {
    thinkData.items.unshift({ id: crypto.randomUUID(), blocks, createdAt: Date.now() });
    thinkData.draft = null; await persistThink();
    thinkInput.replaceChildren(); renderThoughts(); thinkInput.focus();
  } catch (error) { thinkData = previous; setStatus(`Couldn't save thought: ${error}`); }
  finally { savingThought = false; thinkInput.contentEditable = 'true'; thinkInput.focus(); }
});
async function initThink() {
  switchHome('think');
  if (!window.__TAURI__) { renderThoughts(); return; }
  thinkInput.contentEditable = 'false';
  thinkData = await invokeThink('get_think_data');
  if (thinkData.draft?.editing) {
    thinkData.editDrafts ||= {}; thinkData.editDrafts[thinkData.draft.editing] = thinkData.draft.blocks; thinkData.draft = null;
    await persistThink();
  }
  if (thinkData.draft) renderBlocks(thinkInput, thinkData.draft.blocks);
  thinkReady = true; thinkInput.contentEditable = 'true';
  renderThoughts(); renderList();
}
window.addEventListener('resize', () => {
  document.querySelectorAll('.thought-card:not(.expanded):not(.is-editing)').forEach(card => {
    const body = card.querySelector('.thought-content'); const truncated = body.clientHeight >= 131 && body.scrollHeight > body.clientHeight + 2; card.querySelector('.thought-expand').hidden = !truncated; card.classList.toggle('is-truncated', truncated);
  });
});
initThink().catch(error => setStatus(`Couldn't load thoughts: ${error}`));

const attachmentsButton = document.getElementById('composer-attachments');
const attachmentsMenu = document.getElementById('composer-attachments-menu');
function closeAttachments() { attachmentsMenu.classList.add('hidden'); attachmentsButton.setAttribute('aria-expanded', 'false'); }
attachmentsButton.addEventListener('click', () => {
  const open = attachmentsMenu.classList.toggle('hidden') === false;
  attachmentsButton.setAttribute('aria-expanded', String(open));
  if (open) document.getElementById('attach-image').focus();
});
attachmentsMenu.addEventListener('click', closeAttachments);
document.addEventListener('click', event => { if (!event.target.closest('.composer-attachment-shell')) closeAttachments(); });
attachmentsMenu.addEventListener('keydown', event => { if (event.key === 'Escape') { closeAttachments(); attachmentsButton.focus(); } });
