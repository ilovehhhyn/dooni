/* Batch draft writes; explicit actions keep ordered, immutable snapshots. */
function createDraftSaver(snapshot, write, onError, draftSnapshot = snapshot, writeDraft = write) {
  let idleTimer;
  let deadlineTimer;
  let pending = false;
  let failed = false;
  let queue = Promise.resolve();

  function save(draftOnly = false) {
    clearTimeout(idleTimer);
    clearTimeout(deadlineTimer);
    deadlineTimer = undefined;
    pending = false;
    const data = draftOnly ? draftSnapshot() : snapshot();
    const persist = draftOnly ? writeDraft : write;
    queue = queue.catch(() => {}).then(() => persist(data)).then(
      () => { failed = false; },
      error => { failed = true; throw error; },
    );
    return queue;
  }

  function flush() {
    return pending || failed ? save(true) : queue;
  }

  function autosave() {
    flush().catch(onError);
  }

  function schedule() {
    pending = true;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(autosave, 250);
    // Schedule a snapshot every second even during continuous typing.
    deadlineTimer ??= setTimeout(autosave, 1000);
  }

  return { save, schedule, flush };
}

if (typeof module !== 'undefined') module.exports = { createDraftSaver };
