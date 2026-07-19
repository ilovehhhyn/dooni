// House-manager: SVG castle where each window rect is a "slot".
// - orange = running   - light gray = idle known   - dark = empty
// Toggle to list view for rename + explicit focus.

const els = {
  toggle:  document.getElementById("mgr-toggle"),
  castleSvg: document.getElementById("castle-svg"),
  empty:   document.getElementById("mgr-empty"),
  viewCastle: document.getElementById("mgr-castle"),
  viewList:   document.getElementById("mgr-list"),
  listUl:  document.getElementById("mgr-list-ul"),
  tooltip: document.getElementById("mgr-tooltip"),
  status:  document.getElementById("mgr-status"),
  retentionDays: document.getElementById("retention-days"),
  retentionSave: document.getElementById("retention-save"),
};

const slots = Array.from(els.castleSvg.querySelectorAll("[data-slot]"));
const CASTLE_MAX = slots.length;

let sessions = [];
let view = "castle";

function setStatus(s) { els.status.textContent = s; }

function paintSlot(slot, s) {
  slot.classList.remove("idle", "running", "session");
  if (s) {
    slot.classList.add("session", s.running ? "running" : "idle");
    slot.__session = s;
  } else {
    slot.__session = null;
  }
}

function renderCastle() {
  const visible = sessions.slice(0, CASTLE_MAX);
  slots.forEach((slot, i) => paintSlot(slot, visible[i]));
  els.empty.classList.toggle("hidden", sessions.length !== 0);
}

function renderList() {
  els.listUl.innerHTML = "";
  if (sessions.length === 0) {
    const li = document.createElement("li");
    li.textContent = "no chat sessions yet";
    li.style.opacity = "0.5";
    els.listUl.appendChild(li);
    return;
  }
  for (const s of sessions) {
    const li = document.createElement("li");

    const dot = document.createElement("span");
    dot.className = "dot " + (s.running ? "running" : "idle");
    li.appendChild(dot);

    const agent = document.createElement("span");
    agent.className = "list-agent";
    agent.textContent = s.agent;
    li.appendChild(agent);

    const title = document.createElement("span");
    title.className = "list-title";
    title.contentEditable = "true";
    title.spellcheck = false;
    title.textContent = s.title;
    title.addEventListener("keydown", (e) => {
      if (e.key === "Enter") { e.preventDefault(); title.blur(); }
    });
    title.addEventListener("blur", async () => {
      const nv = title.textContent.trim();
      if (!nv || nv === s.title) { title.textContent = s.title; return; }
      try {
        await window.__TAURI__.core.invoke("rename_session", {
          sessionId: s.session_id,
          title: nv,
        });
        s.title = nv;
      } catch (e) { setStatus("rename err: " + e); }
    });
    li.appendChild(title);

    const btn = document.createElement("button");
    btn.className = "list-focus";
    btn.textContent = "focus";
    btn.addEventListener("click", () => focusSession(s.session_id));
    li.appendChild(btn);

    els.listUl.appendChild(li);
  }
}

function renderAll() {
  if (view === "castle") renderCastle(); else renderList();
}

async function focusSession(id) {
  try {
    const matched = await window.__TAURI__.core.invoke("focus_session", { sessionId: id });
    setStatus(matched ? "focused terminal tab" : "activated terminal app (no exact tab match)");
  } catch (e) { setStatus("focus err: " + e); }
}

function showTip(e, s) {
  const last = s.last_active ? new Date(s.last_active * 1000).toLocaleTimeString() : "—";
  els.tooltip.innerHTML =
    `<div><b>${escapeHtml(s.title)}</b></div>` +
    `<div>${escapeHtml(s.agent)} · ${s.running ? "running" : "idle"}</div>` +
    (s.project_dir ? `<div>${escapeHtml(s.project_dir)}</div>` : "") +
    `<div>last: ${last}</div>`;
  els.tooltip.classList.remove("hidden");
  moveTip(e);
}
function moveTip(e) {
  els.tooltip.style.left = (e.clientX + 12) + "px";
  els.tooltip.style.top  = (e.clientY + 12) + "px";
}
function hideTip() { els.tooltip.classList.add("hidden"); }

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => (
    {"&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;"}[c]
  ));
}

async function refresh() {
  try {
    sessions = await window.__TAURI__.core.invoke("get_sessions");
    renderAll();
    setStatus(`${sessions.length} session${sessions.length===1?"":"s"} · ${new Date().toLocaleTimeString()}`);
  } catch (e) { setStatus("load err: " + e); }
}

function setupLockButton() {
  const btn = document.getElementById("lock-btn");
  if (!btn) return;
  let pinned = true;
  const paint = () => { btn.textContent = pinned ? "🔒" : "🔓"; };
  paint();
  btn.addEventListener("click", async () => {
    pinned = !pinned;
    paint();
    try {
      const w = window.__TAURI__ && window.__TAURI__.window;
      const getter = w && (w.getCurrentWindow || w.getCurrent);
      const cur = getter ? getter.call(w) : null;
      if (cur && cur.setAlwaysOnTop) await cur.setAlwaysOnTop(pinned);
    } catch (e) { console.error("lock err", e); }
  });
}

async function init() {
  if (!window.__TAURI__) { setStatus("NO __TAURI__"); return; }
  setupLockButton();

  try {
    const config = await window.__TAURI__.core.invoke("get_config");
    els.retentionDays.value = config.terminal_retention_days || 5;
  } catch (e) { setStatus("config err: " + e); }

  els.retentionSave.addEventListener("click", async () => {
    const days = Number(els.retentionDays.value);
    if (!Number.isInteger(days) || days < 1 || days > 3650) {
      setStatus("retention must be 1–3650 days");
      return;
    }
    try {
      const config = await window.__TAURI__.core.invoke(
        "set_terminal_retention_days", { days }
      );
      els.retentionDays.value = config.terminal_retention_days;
      setStatus(`idle entries kept for ${config.terminal_retention_days} days`);
    } catch (e) { setStatus("retention err: " + e); }
  });

  // Delegated slot handlers so we don't re-bind on every render.
  els.castleSvg.addEventListener("mousemove", (e) => {
    const target = e.target;
    if (target && target.__session) moveTip(e);
  });
  els.castleSvg.addEventListener("mouseover", (e) => {
    const target = e.target;
    if (target && target.__session) showTip(e, target.__session);
  });
  els.castleSvg.addEventListener("mouseout", (e) => {
    const target = e.target;
    if (target && target.__session) hideTip();
  });
  els.castleSvg.addEventListener("click", (e) => {
    const target = e.target;
    if (target && target.__session) focusSession(target.__session.session_id);
  });

  els.toggle.addEventListener("click", () => {
    view = view === "castle" ? "list" : "castle";
    els.toggle.textContent = view === "castle" ? "list" : "castle";
    els.viewCastle.classList.toggle("hidden", view !== "castle");
    els.viewList.classList.toggle("hidden", view !== "list");
    renderAll();
  });

  await refresh();

  try {
    await window.__TAURI__.event.listen("sessions-updated", (evt) => {
      const p = evt.payload || {};
      if (Array.isArray(p.sessions)) {
        sessions = p.sessions;
        renderAll();
        setStatus(`${sessions.length} session${sessions.length===1?"":"s"} · ${new Date().toLocaleTimeString()}`);
      }
    });
  } catch (e) { setStatus("listen err: " + e); }

  setInterval(refresh, 10000);
}

init().catch(e => setStatus("init err: " + e));
