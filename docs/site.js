const dialog = document.querySelector('#demo-dialog');
const video = dialog.querySelector('video');
let startAt = 0;
video.addEventListener('loadedmetadata', () => { video.currentTime = startAt; });
document.querySelectorAll('[data-time]').forEach(button => {
  button.addEventListener('click', () => {
    startAt = Number(button.dataset.time);
    dialog.showModal();
    if (video.readyState >= 1) video.currentTime = startAt;
    video.play().catch(() => { /* Native controls remain available. */ });
  });
});
document.querySelector('#close-demo').addEventListener('click', () => dialog.close());
dialog.addEventListener('close', () => video.pause());
dialog.addEventListener('click', event => { if (event.target === dialog) { const r = dialog.getBoundingClientRect(); if (event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom) dialog.close(); } });
document.querySelector('#copy').addEventListener('click', async () => {
  const status = document.querySelector('#copy-status');
  try {
    await navigator.clipboard.writeText(document.querySelector('#commands').textContent);
    status.textContent = 'Copied. Paste into Terminal to install.';
  } catch {
    status.textContent = 'Select the commands above and copy them manually.';
  }
});
