const { test } = require('node:test');
const assert = require('node:assert/strict');
const { createDraftSaver } = require('../src/draft-saver.js');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

test('typing batches snapshots and writes, including large attachments', async () => {
  let snapshots = 0;
  const data = { text: '', image: 'x'.repeat(1024 * 1024) };
  const writes = [];
  const saver = createDraftSaver(() => { snapshots++; return structuredClone(data); }, async value => writes.push(value), assert.fail);
  for (let i = 0; i < 100; i++) { data.text += 'a'; saver.schedule(); }
  assert.equal(snapshots, 0);
  await delay(300);
  assert.equal(snapshots, 1);
  assert.equal(writes.length, 1);
  assert.equal(writes[0].text.length, 100);
});

test('explicit save cancels stale draft timer and preserves snapshot ordering', async () => {
  let text = 'draft';
  const writes = [];
  const saver = createDraftSaver(() => text, async value => { await delay(10); writes.push(value); }, assert.fail);
  saver.schedule();
  text = 'submitted';
  const first = saver.save();
  text = 'next draft'; saver.schedule();
  await saver.flush(); await first; await delay(300);
  assert.deepEqual(writes, ['submitted', 'next draft']);
});

test('a failed write does not poison subsequent saves', async () => {
  let fail = true;
  const writes = [];
  const saver = createDraftSaver(() => 'latest', async value => { if (fail) throw new Error('disk'); writes.push(value); }, () => {});
  await assert.rejects(saver.save(), /disk/);
  fail = false;
  await saver.flush();
  assert.deepEqual(writes, ['latest']);
});

test('continuous typing has a bounded autosave delay', async () => {
  let writes = 0;
  const saver = createDraftSaver(() => 'draft', async () => writes++, assert.fail);
  const interval = setInterval(() => saver.schedule(), 50);
  try { await delay(1150); assert.ok(writes >= 1); }
  finally { clearInterval(interval); await saver.flush(); }
});

test('autosave sends only drafts; explicit actions save the full collection in order', async () => {
  const writes = [];
  let fullSnapshots = 0;
  const saver = createDraftSaver(
    () => { fullSnapshots++; return { items: ['large image'], draft: null }; },
    async data => writes.push(['full', data]), assert.fail,
    () => ({ draft: 'typing' }), async data => writes.push(['draft', data]),
  );
  saver.schedule(); await saver.flush();
  assert.equal(fullSnapshots, 0);
  assert.deepEqual(writes, [['draft', { draft: 'typing' }]]);
  await saver.save();
  assert.equal(fullSnapshots, 1);
  assert.equal(writes[1][0], 'full');
});
