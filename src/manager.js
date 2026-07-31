const els = {
  list: document.getElementById("mgr-list-ul"),
  count: document.getElementById("session-count"),
  status: document.getElementById("mgr-status"),
  settings: document.getElementById("settings-panel"),
  settingsButton: document.getElementById("settings-btn"),
  settingsClose: document.getElementById("settings-close"),
  codexDetail: document.getElementById("codex-runtime"),
  codexState: document.getElementById("codex-runtime-state"),
  codexAction: document.getElementById("codex-runtime-action"),
  anthropicDetail: document.getElementById("anthropic-runtime"),
  anthropicKey: document.getElementById("settings-anthropic-key"),
  runtimeSave: document.getElementById("runtime-save"),
};

const ICONS = {
  edit: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M12.6 2.6a2 2 0 0 1 2.8 2.8L6 14.8l-4 .9.9-4L12.6 2.6zM10.8 4.4l2.8 2.8"></path></svg>',
  open: '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5 11L11 5M7 5h4v4"></path></svg>',
};

let sessions = [];
let config = null;
let selectedRuntime = "codex";
let codexStatus = { installed: false, authenticated: false };

function setStatus(message) {
  els.status.textContent = message || "";
  els.status.classList.toggle("visible", !!message);
}

function renderList() {
  els.list.innerHTML = "";
  els.count.textContent = `${sessions.length} chat${sessions.length === 1 ? "" : "s"}`;
  if (!sessions.length) {
    const empty = document.createElement("li");
    empty.className = "list-empty";
    empty.textContent = "no chats";
    els.list.appendChild(empty);
    return;
  }

  sessions.forEach((session) => {
    const item = document.createElement("li");
    item.className = "session-row";

    const dot = document.createElement("span");
    dot.className = `session-dot ${session.running ? "running" : "idle"}`;

    const identity = document.createElement("div");
    identity.className = "session-row-identity";

    const title = document.createElement("span");
    title.className = "list-title";
    title.textContent = session.title;
    title.tabIndex = 0;
    title.setAttribute("role", "button");
    title.addEventListener("click", () => {
      if (!title.isContentEditable) focusSession(session.session_id);
    });
    title.addEventListener("keydown", (event) => {
      if (!title.isContentEditable && (event.key === "Enter" || event.key === " ")) {
        event.preventDefault();
        focusSession(session.session_id);
      }
    });

    const meta = document.createElement("span");
    meta.className = "list-meta";
    const agent = session.agent === "codex"
      ? "Codex"
      : session.agent === "claude"
        ? "Claude"
        : "Unknown";
    meta.textContent = [session.project_name, agent].filter(Boolean).join(" | ");
    identity.append(title, meta);

    const rename = iconButton("edit", `Rename ${session.title}`);
    rename.addEventListener("click", () => beginRename(title, session));

    const launch = iconButton("open", `Open notes for ${session.title}`);
    launch.classList.add("bare-arrow");
    launch.addEventListener("click", () => launchSession(session.session_id));

    item.append(dot, identity, rename, launch);
    els.list.appendChild(item);
  });
}

function iconButton(icon, label) {
  const button = document.createElement("button");
  button.className = "icon-button row-action";
  button.type = "button";
  button.setAttribute("aria-label", label);
  button.innerHTML = ICONS[icon];
  return button;
}

async function focusSession(sessionId) {
  try {
    const matched = await window.__TAURI__.core.invoke("focus_session", { sessionId });
    if (!matched) setStatus("chat is unavailable");
  } catch (error) {
    setStatus(`focus error: ${error}`);
  }
}

async function launchSession(sessionId) {
  try {
    await window.__TAURI__.core.invoke("launch_session_window", { sessionId });
  } catch (error) {
    setStatus(`open error: ${error}`);
  }
}

function beginRename(title, session) {
  if (title.isContentEditable) return;
  title.removeAttribute("role");
  title.removeAttribute("tabindex");
  title.contentEditable = "true";
  title.classList.add("renaming");
  title.focus();
  document.execCommand("selectAll", false, null);

  const handleKey = (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      title.blur();
    } else if (event.key === "Escape") {
      title.textContent = session.title;
      title.blur();
    }
  };
  const finish = async () => {
    title.removeEventListener("blur", finish);
    title.removeEventListener("keydown", handleKey);
    const next = title.textContent.trim();
    title.contentEditable = "false";
    title.classList.remove("renaming");
    title.setAttribute("role", "button");
    title.tabIndex = 0;
    if (!next || next === session.title) {
      title.textContent = session.title;
      return;
    }
    try {
      const updatedTitle = await window.__TAURI__.core.invoke("rename_session", {
        sessionId: session.session_id,
        title: next,
      });
      session.title = updatedTitle;
      title.textContent = updatedTitle;
    } catch (error) {
      title.textContent = session.title;
      setStatus(`rename error: ${error}`);
    }
  };
  title.addEventListener("keydown", handleKey);
  title.addEventListener("blur", finish);
}

async function refreshSessions() {
  try {
    sessions = await window.__TAURI__.core.invoke("get_sessions");
    renderList();
  } catch (error) {
    setStatus(`load error: ${error}`);
  }
}

function setupPinButton() {
  const button = document.getElementById("lock-btn");
  let pinned = false;
  button.addEventListener("click", async () => {
    pinned = !pinned;
    button.setAttribute("aria-pressed", String(pinned));
    button.title = pinned ? "Unpin window" : "Pin window";
    try {
      const api = window.__TAURI__.window;
      const getter = api.getCurrentWindow || api.getCurrent;
      const current = getter ? getter.call(api) : null;
      if (current?.setAlwaysOnTop) await current.setAlwaysOnTop(pinned);
    } catch (error) {
      setStatus(`pin error: ${error}`);
    }
  });
}

function paintRuntime() {
  document.querySelectorAll(".runtime-option").forEach((option) => {
    option.classList.toggle("selected", option.dataset.runtime === selectedRuntime);
  });
  els.codexDetail.classList.toggle("hidden", selectedRuntime !== "codex");
  els.anthropicDetail.classList.toggle("hidden", selectedRuntime !== "anthropic");
}

function paintCodexStatus() {
  if (!codexStatus.installed) {
    els.codexState.textContent = "not installed";
    els.codexAction.textContent = "install";
  } else if (!codexStatus.authenticated) {
    els.codexState.textContent = "not connected";
    els.codexAction.textContent = "sign in";
  } else {
    els.codexState.textContent = "connected";
    els.codexAction.textContent = "";
  }
  els.codexAction.classList.toggle("hidden", codexStatus.authenticated);
}

async function refreshSettings() {
  try {
    const [nextConfig, nextCodexStatus] = await Promise.all([
      window.__TAURI__.core.invoke("get_config"),
      window.__TAURI__.core.invoke("get_codex_status"),
    ]);
    config = nextConfig;
    codexStatus = nextCodexStatus;
    selectedRuntime = config.runtime_provider || "codex";
    paintRuntime();
    paintCodexStatus();
  } catch (error) {
    setStatus(`settings error: ${error}`);
  }
}

async function handleCodexAction() {
  try {
    if (!codexStatus.installed) {
      await window.__TAURI__.core.invoke("open_codex_install");
    } else if (!codexStatus.authenticated) {
      els.codexAction.textContent = "opening…";
      await window.__TAURI__.core.invoke("start_codex_login");
      setTimeout(refreshSettings, 1500);
    }
  } catch (error) {
    setStatus(`Codex error: ${error}`);
  }
}

async function saveRuntime() {
  try {
    if (selectedRuntime === "codex") {
      codexStatus = await window.__TAURI__.core.invoke("get_codex_status");
      paintCodexStatus();
      if (!codexStatus.authenticated) {
        setStatus("connect Codex first");
        return;
      }
    }
    const apiKey = els.anthropicKey.value.trim();
    config = await window.__TAURI__.core.invoke("update_runtime_provider", {
      runtimeProvider: selectedRuntime,
      apiKey: apiKey || null,
    });
    els.anthropicKey.value = "";
    els.settings.classList.add("hidden");
  } catch (error) {
    setStatus(`runtime error: ${error}`);
  }
}

function setupSettings() {
  paintRuntime();
  els.settingsButton.addEventListener("click", async () => {
    els.settings.classList.remove("hidden");
    await refreshSettings();
  });
  els.settingsClose.addEventListener("click", () => els.settings.classList.add("hidden"));
  document.querySelectorAll(".runtime-option").forEach((option) => {
    option.addEventListener("click", () => {
      selectedRuntime = option.dataset.runtime;
      paintRuntime();
    });
  });
  els.codexAction.addEventListener("click", handleCodexAction);
  els.runtimeSave.addEventListener("click", saveRuntime);
}

async function init() {
  setupSettings();
  if (!window.__TAURI__) {
    renderList();
    return;
  }
  setupPinButton();
  await refreshSessions();
  await window.__TAURI__.event.listen("sessions-updated", (event) => {
    const payload = event.payload || {};
    if (Array.isArray(payload.sessions)) {
      sessions = payload.sessions;
      renderList();
    }
  });
  setInterval(refreshSessions, 10000);
  setInterval(() => {
    if (!els.settings.classList.contains("hidden") && !codexStatus.authenticated) {
      refreshSettings();
    }
  }, 3000);
}

init().catch((error) => setStatus(`init error: ${error}`));
