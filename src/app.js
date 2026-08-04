// The main window handles onboarding; launched chat windows show the user's
// source-history prompts and editable future-prompt scratch notes.

const screens = {
  onboard: document.getElementById("screen-onboard"),
  memo: document.getElementById("screen-memo"),
};

const els = {
  status: document.getElementById("status"),
  asked: document.getElementById("asked-prompts"),
  prompts: document.getElementById("future-prompts"),
  promptForm: document.getElementById("prompt-form"),
  promptInput: document.getElementById("prompt-input"),
  memoTitle: document.getElementById("memo-title"),
  askedCount: document.getElementById("asked-count"),
  promptCount: document.getElementById("prompt-count"),
};

let onboarded = false;
let userName = "";
let sessionId = null;
let askedPrompts = [];
let askedPromptTimestamps = [];
let futurePrompts = [];
const ONBOARDING_SUCCESS_MILLIS = 2000;

function showScreen(name) {
  for (const [key, el] of Object.entries(screens)) {
    if (el) el.classList.toggle("hidden", key !== name);
  }
}

function chooseScreen() {
  if (sessionId) return showScreen("memo");
  if (!onboarded) return showScreen("onboard");
  window.location.replace("manager.html");
}

function readOwnLabel() {
  try {
    const api = window.__TAURI__.window;
    const getter = api.getCurrentWindow || api.getCurrent;
    return getter ? getter.call(api).label || "main" : "main";
  } catch (_) {
    return "main";
  }
}

function setStatus(text) {
  if (!els.status) return;
  els.status.textContent = text;
  els.status.classList.toggle("visible", !!text);
}

function wiggle() {
  document.querySelectorAll(".blob").forEach((blob) => {
    blob.classList.remove("wiggle");
    void blob.offsetWidth;
    blob.classList.add("wiggle");
  });
}

function setupLockButton() {
  const btn = document.getElementById("lock-btn");
  if (!btn) return;
  let pinned = false;
  const paint = () => {
    btn.setAttribute("aria-pressed", String(pinned));
    btn.title = pinned ? "Unpin window" : "Pin window";
  };
  paint();
  btn.addEventListener("click", async () => {
    pinned = !pinned;
    paint();
    try {
      const api = window.__TAURI__.window;
      const getter = api.getCurrentWindow || api.getCurrent;
      const current = getter ? getter.call(api) : null;
      if (current?.setAlwaysOnTop) await current.setAlwaysOnTop(pinned);
    } catch (error) {
      console.error("pin error", error);
    }
  });
}

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch (_) {
    const fallback = document.createElement("textarea");
    fallback.value = text;
    fallback.setAttribute("readonly", "");
    fallback.style.position = "fixed";
    fallback.style.opacity = "0";
    document.body.appendChild(fallback);
    fallback.select();
    const copied = document.execCommand("copy");
    fallback.remove();
    if (!copied) throw new Error("clipboard unavailable");
  }
}

function promptText(text, onSave) {
  const value = document.createElement("span");
  value.className = "prompt-text";
  value.setAttribute("role", "button");
  value.setAttribute("aria-label", "Copy thought");
  value.tabIndex = 0;
  value.spellcheck = true;
  value.textContent = text;

  const copy = async () => {
    if (value.isContentEditable) return;
    try {
      await copyText(value.textContent);
      setStatus("copied");
      window.setTimeout(() => setStatus(""), 1200);
    } catch (error) {
      setStatus(`copy error: ${error}`);
    }
  };
  value.addEventListener("click", copy);
  value.addEventListener("keydown", (event) => {
    if (value.isContentEditable && event.key === "Enter") {
      event.preventDefault();
      value.blur();
    } else if (!value.isContentEditable && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault();
      copy();
    }
  });
  value.addEventListener("blur", () => {
    if (!value.isContentEditable) return;
    value.contentEditable = "false";
    value.classList.remove("editing");
    value.setAttribute("role", "button");
    value.setAttribute("aria-label", "Copy thought");
    value.tabIndex = 0;
    onSave(value.textContent.trim(), value);
  });
  return value;
}

function beginPromptEdit(value) {
  if (value.isContentEditable) return;
  value.contentEditable = "true";
  value.classList.add("editing");
  value.removeAttribute("role");
  value.removeAttribute("aria-label");
  value.removeAttribute("tabindex");
  value.focus();
  document.execCommand("selectAll", false, null);
}

async function persistPrompts() {
  await window.__TAURI__.core.invoke("save_future_prompts", {
    sessionId,
    prompts: futurePrompts,
  });
}

function renderAskedPrompts() {
  els.asked.innerHTML = "";
  let previousDay = null;
  askedPrompts.forEach((prompt, index) => {
    const day = promptDay(askedPromptTimestamps[index]);
    if (day.key !== previousDay) {
      const divider = document.createElement("li");
      divider.className = "asked-day-divider";
      const date = document.createElement("time");
      date.textContent = day.label;
      if (day.dateTime) date.dateTime = day.dateTime;
      divider.appendChild(date);
      els.asked.appendChild(divider);
      previousDay = day.key;
    }
    const item = document.createElement("li");
    item.className = "asked-prompt";
    const text = document.createElement("span");
    text.className = "asked-prompt-text";
    text.textContent = prompt;

    const locate = document.createElement("button");
    locate.className = "locate-prompt";
    locate.type = "button";
    locate.title = "Find in original chat";
    locate.setAttribute("aria-label", "Find this prompt in the original chat");
    locate.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5 11 11 5M6 5h5v5"></path></svg>';
    locate.addEventListener("click", async () => {
      locate.disabled = true;
      setStatus("opening chat…");
      try {
        const result = await window.__TAURI__.core.invoke("locate_asked_prompt", {
          sessionId,
          promptIndex: index,
        });
        setStatus(result.message || "chat opened");
        window.setTimeout(() => setStatus(""), 4200);
      } catch (error) {
        setStatus(`could not locate prompt: ${error}`);
      } finally {
        locate.disabled = false;
      }
    });

    item.append(text, locate);
    els.asked.appendChild(item);
  });
  els.askedCount.textContent = askedPrompts.length;
}

function promptDay(timestamp) {
  const date = timestamp ? new Date(timestamp) : null;
  if (!date || Number.isNaN(date.getTime())) {
    return { key: "earlier", label: "Earlier", dateTime: "" };
  }
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return {
    key: `${year}-${month}-${day}`,
    label: new Intl.DateTimeFormat(undefined, {
      year: "numeric",
      month: "long",
      day: "numeric",
    }).format(date),
    dateTime: `${year}-${month}-${day}`,
  };
}

function renderPrompts() {
  els.prompts.innerHTML = "";
  if (futurePrompts.length) {
    futurePrompts.forEach((prompt, index) => {
      const item = document.createElement("li");
      item.className = prompt.done ? "prompt-item done" : "prompt-item";

      const check = document.createElement("button");
      check.className = "prompt-check";
      check.type = "button";
      check.setAttribute("aria-label", prompt.done ? "Mark not done" : "Mark done");
      check.textContent = prompt.done ? "✓" : "";
      check.addEventListener("click", async () => {
        prompt.done = !prompt.done;
        renderPrompts();
        try { await persistPrompts(); } catch (error) { setStatus(`save error: ${error}`); }
      });

      const text = promptText(prompt.text, async (next, target) => {
        if (!next) {
          futurePrompts.splice(index, 1);
          renderPrompts();
        } else {
          prompt.text = next;
          target.textContent = next;
        }
        try { await persistPrompts(); } catch (error) { setStatus(`save error: ${error}`); }
      });

      const edit = document.createElement("button");
      edit.className = "edit-line";
      edit.type = "button";
      edit.setAttribute("aria-label", "Edit thought");
      edit.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M12.6 2.6a2 2 0 0 1 2.8 2.8L6 14.8l-4 .9.9-4L12.6 2.6zM10.8 4.4l2.8 2.8"></path></svg>';
      edit.addEventListener("click", () => beginPromptEdit(text));

      const remove = document.createElement("button");
      remove.className = "remove-line";
      remove.type = "button";
      remove.setAttribute("aria-label", "Remove prompt");
      remove.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8"></path></svg>';
      remove.addEventListener("click", async () => {
        futurePrompts.splice(index, 1);
        renderPrompts();
        try { await persistPrompts(); } catch (error) { setStatus(`save error: ${error}`); }
      });

      item.append(check, text, remove, edit);
      els.prompts.appendChild(item);
    });
  }
  els.promptCount.textContent = futurePrompts.filter((prompt) => !prompt.done).length;
}

function setupTabs() {
  document.querySelectorAll(".tab").forEach((tab) => {
    tab.addEventListener("click", () => {
      const target = tab.dataset.tab;
      document.querySelectorAll(".tab").forEach((item) => item.classList.toggle("active", item === tab));
      document.querySelectorAll(".panel").forEach((panel) => {
        panel.classList.toggle("active", panel.id === `panel-${target}`);
      });
      if (target === "future") requestAnimationFrame(() => els.promptInput.focus());
    });
  });
}

function newPromptId() {
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

async function initSession(invoke, listen) {
  const data = await invoke("get_session", { sessionId });
  if (!data) {
    setStatus("This chat is no longer tracked.");
    chooseScreen();
    return;
  }
  askedPrompts = Array.isArray(data.asked_prompts) ? data.asked_prompts : [];
  askedPromptTimestamps = Array.isArray(data.asked_prompt_timestamps)
    ? data.asked_prompt_timestamps
    : [];
  futurePrompts = Array.isArray(data.future_prompts) ? data.future_prompts : [];
  els.memoTitle.textContent = data.title || "untitled chat";
  renderAskedPrompts();
  renderPrompts();
  setupTabs();

  els.promptForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const text = els.promptInput.value.trim();
    if (!text) return;
    futurePrompts.push({ id: newPromptId(), text, done: false });
    els.promptInput.value = "";
    resizePromptInput();
    renderPrompts();
    try { await persistPrompts(); } catch (error) { setStatus(`save error: ${error}`); }
  });
  els.promptInput.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.isComposing) return;
    if (event.metaKey || event.ctrlKey) {
      event.preventDefault();
      const start = els.promptInput.selectionStart;
      const end = els.promptInput.selectionEnd;
      els.promptInput.setRangeText("\n", start, end, "end");
      resizePromptInput();
      return;
    }
    event.preventDefault();
    els.promptForm.requestSubmit();
  });
  els.promptInput.addEventListener("input", resizePromptInput);
  await listen("asked-prompts-updated", (event) => {
    const payload = event.payload || {};
    if (payload.session_id !== sessionId) return;
    askedPrompts = Array.isArray(payload.asked_prompts) ? payload.asked_prompts : [];
    askedPromptTimestamps = Array.isArray(payload.asked_prompt_timestamps)
      ? payload.asked_prompt_timestamps
      : [];
    if (payload.title) els.memoTitle.textContent = payload.title;
    renderAskedPrompts();
  });
  chooseScreen();
  requestAnimationFrame(() => els.promptInput.focus());
}

async function initMain(invoke, listen) {
  const codexAction = document.getElementById("ob-codex-action");
  const codexStatusText = document.getElementById("ob-codex-status");
  const codexDot = document.getElementById("ob-codex-dot");
  const anthropicToggle = document.getElementById("ob-use-anthropic");
  const anthropicPanel = document.getElementById("ob-anthropic-panel");
  const runtimeStep = document.getElementById("ob-runtime-step");
  const claudeStep = document.getElementById("ob-claude-step");
  const claudeAction = document.getElementById("ob-claude-action");
  const claudeSkip = document.getElementById("ob-claude-skip");
  const claudeStatusText = document.getElementById("ob-claude-status");
  const claudeDot = document.getElementById("ob-claude-dot");
  const error = document.getElementById("ob-error");
  let codexStatus = { installed: false, authenticated: false };
  let claudeAccess = { installed: false, authorized: false };
  let pendingRuntimeProvider = "codex";
  let pendingApiKey = "";

  async function finishOnboarding() {
    const name = document.getElementById("ob-name").value.trim();
    if (!name) return void (error.textContent = "name required");
    try {
      await invoke("save_config", {
        apiKey: pendingApiKey,
        runtimeProvider: pendingRuntimeProvider,
        name,
        agents: ["claude", "codex"],
      });
      onboarded = true;
      userName = name;
      wiggle();
      setTimeout(() => window.location.replace("manager.html"), ONBOARDING_SUCCESS_MILLIS);
    } catch (saveError) {
      error.textContent = `save error: ${saveError}`;
    }
  }

  async function showClaudeStep(runtimeProvider, apiKey = "") {
    const name = document.getElementById("ob-name").value.trim();
    if (!name) return void (error.textContent = "name required");
    pendingRuntimeProvider = runtimeProvider;
    pendingApiKey = apiKey;
    error.textContent = "";
    runtimeStep.classList.add("hidden");
    claudeStep.classList.remove("hidden");
    await refreshClaudeAccess();
  }

  function paintCodexStatus() {
    codexDot.className = "auth-dot";
    codexAction.disabled = false;
    if (!codexStatus.installed) {
      codexDot.classList.add("missing");
      codexStatusText.textContent = "not installed";
      codexAction.textContent = "install Codex CLI";
    } else if (!codexStatus.authenticated) {
      codexStatusText.textContent = "not connected";
      codexAction.textContent = "sign in with Codex";
    } else {
      codexDot.classList.add("connected");
      codexStatusText.textContent = "connected";
      codexAction.textContent = "continue with Codex";
    }
  }

  async function refreshCodexStatus() {
    try {
      codexStatus = await invoke("get_codex_status");
      paintCodexStatus();
    } catch (statusError) {
      codexAction.disabled = false;
      codexAction.textContent = "check Codex again";
      error.textContent = `Codex check failed: ${statusError}`;
    }
  }

  function paintClaudeAccess() {
    claudeDot.className = "auth-dot";
    claudeAction.disabled = false;
    if (!claudeAccess.installed) {
      claudeDot.classList.add("missing");
      claudeStatusText.textContent = "not installed";
      claudeAction.textContent = "install Claude Desktop";
    } else if (!claudeAccess.authorized) {
      claudeStatusText.textContent = "not connected";
      claudeAction.textContent = "allow Claude Desktop";
    } else {
      claudeDot.classList.add("connected");
      claudeStatusText.textContent = "connected";
      claudeAction.textContent = "continue";
    }
  }

  async function refreshClaudeAccess() {
    try {
      claudeAccess = await invoke("get_claude_desktop_access_status");
      paintClaudeAccess();
    } catch (statusError) {
      claudeAction.disabled = false;
      claudeAction.textContent = "check Claude again";
      error.textContent = `Claude check failed: ${statusError}`;
    }
  }

  codexAction.addEventListener("click", async () => {
    error.textContent = "";
    try {
      if (!codexStatus.installed) {
        await invoke("open_codex_install");
        return;
      }
      if (!codexStatus.authenticated) {
        codexAction.disabled = true;
        codexAction.textContent = "opening sign in…";
        await invoke("start_codex_login");
        setTimeout(refreshCodexStatus, 1500);
        return;
      }
      await showClaudeStep("codex");
    } catch (actionError) {
      error.textContent = `${actionError}`;
      paintCodexStatus();
    }
  });

  anthropicToggle.addEventListener("click", () => {
    anthropicPanel.classList.toggle("hidden");
  });
  document.getElementById("ob-anthropic-save").addEventListener("click", async () => {
    const apiKey = document.getElementById("ob-anthropic-key").value.trim();
    if (!apiKey) return void (error.textContent = "API key required");
    await showClaudeStep("anthropic", apiKey);
  });
  claudeAction.addEventListener("click", async () => {
    error.textContent = "";
    try {
      if (claudeAccess.authorized) {
        await finishOnboarding();
        return;
      }
      await invoke("open_claude_desktop_access");
      window.setTimeout(refreshClaudeAccess, 1000);
    } catch (actionError) {
      error.textContent = `${actionError}`;
      paintClaudeAccess();
    }
  });
  claudeSkip.addEventListener("click", finishOnboarding);

  await refreshCodexStatus();
  setInterval(() => {
    if (!onboarded && !codexStatus.authenticated) refreshCodexStatus();
  }, 3000);
  setInterval(() => {
    if (!onboarded && !claudeStep.classList.contains("hidden") && !claudeAccess.authorized) {
      refreshClaudeAccess();
    }
  }, 1500);

  chooseScreen();
}

async function init() {
  if (!window.__TAURI__) {
    showScreen("onboard");
    return;
  }
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const label = readOwnLabel();
  if (label.startsWith("session-")) sessionId = label.slice("session-".length);

  const config = await invoke("get_config");
  onboarded = !!config.onboarded;
  userName = config.name || "";
  const onboardingName = document.getElementById("ob-name");
  if (onboardingName && userName) onboardingName.value = userName;
  if (!sessionId && onboarded) {
    window.location.replace("manager.html");
    return;
  }
  setupLockButton();

  if (sessionId) await initSession(invoke, listen);
  else await initMain(invoke, listen);
}

function resizePromptInput() {
  if (!els.promptInput) return;
  els.promptInput.style.height = "auto";
  els.promptInput.style.height = `${Math.min(els.promptInput.scrollHeight, 112)}px`;
}

init().catch((error) => {
  setStatus(`init error: ${error}`);
  showScreen(sessionId ? "memo" : "onboard");
});
