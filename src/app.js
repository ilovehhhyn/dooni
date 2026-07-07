// dooni frontend. Runs in one of two window kinds:
//   - the "main" window: welcome / greeting / onboarding
//   - a "session-<id>" window: dedicated memo for one chat session
// Rendering logic per window is unchanged from the single-window version;
// this file just picks which screens the window is allowed to show.

const screens = {
  welcome:  document.getElementById("screen-welcome"),
  greeting: document.getElementById("screen-greeting"),
  onboard:  document.getElementById("screen-onboard"),
  memo:     document.getElementById("screen-memo"),
};

const status       = document.getElementById("status");
const ul           = document.getElementById("topics");
const toggleBtn    = document.getElementById("toggle");
const greetingText = document.getElementById("greeting-text");
const memoSub      = document.getElementById("memo-sub");

let mode          = "curt";
let onboarded     = false;
let sessionActive = false;
let hasTopics     = false;
let userName      = "";
let windowLabel   = "main";
let sessionId     = null; // null on the main window; string on session windows

function isSessionWindow() { return sessionId !== null; }

function showScreen(name) {
  for (const [k, el] of Object.entries(screens)) {
    if (!el) continue;
    el.classList.toggle("hidden", k !== name);
  }
}

function pickGreeting(name) {
  const options = [
    `dooni says hi to ${name}`,
    `dooni hopes ${name} is having a good day`,
    `dooni hopes ${name} is drinking water`,
    `dooni missed ${name}`,
    `hi ${name}, dooni is here`,
  ];
  return options[Math.floor(Math.random() * options.length)];
}

function chooseScreen() {
  if (isSessionWindow()) {
    showScreen("memo");
    return;
  }
  if (!onboarded) {
    showScreen(sessionActive ? "onboard" : "welcome");
    return;
  }
  showScreen("greeting");
  greetingText.textContent = pickGreeting(userName || "you");
}

function setStatus(s) { if (status) status.textContent = s; }

function render(topics) {
  hasTopics = !!(topics && topics.length);
  ul.innerHTML = "";
  if (!hasTopics) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = "// no topics yet, start chatting";
    ul.appendChild(li);
  } else {
    for (const t of topics) {
      const li = document.createElement("li");
      li.textContent = t;
      ul.appendChild(li);
    }
    ul.parentElement.scrollTop = ul.parentElement.scrollHeight;
  }
}

function wiggleAll() {
  document.querySelectorAll(".blob").forEach(b => {
    b.classList.remove("wiggle");
    void b.offsetWidth;
    b.classList.add("wiggle");
  });
}

function readOwnLabel() {
  try {
    const w = window.__TAURI__.window;
    // Tauri v2 exposes getCurrentWindow; older betas used getCurrent.
    const getter = w.getCurrentWindow || w.getCurrent;
    const cur = getter ? getter.call(w) : null;
    if (cur && cur.label) return cur.label;
  } catch (_) {}
  return "main";
}

async function init() {
  if (!window.__TAURI__) {
    setStatus("NO __TAURI__");
    showScreen("welcome");
    return;
  }
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  windowLabel = readOwnLabel();
  if (windowLabel.startsWith("session-")) {
    sessionId = windowLabel.slice("session-".length);
  }

  const cfg = await invoke("get_config");
  mode      = cfg.mode || "curt";
  onboarded = !!cfg.onboarded;
  userName  = cfg.name || "";
  toggleBtn.textContent = mode;

  // --- Session window: just render memo ---
  if (isSessionWindow()) {
    memoSub.textContent = `session ${sessionId.slice(0, 12)}`;
    try {
      const topics = await invoke("get_topics", { sessionId });
      render(topics);
      setStatus(`${topics.length} entries`);
    } catch (e) { setStatus("invoke err: " + e); }

    try {
      await listen("topics-updated", (evt) => {
        const p = evt.payload || {};
        if (p.session_id !== sessionId) return;
        render(p.topics || []);
        setStatus(`updated ${(p.topics || []).length} @ ${new Date().toLocaleTimeString()}`);
        wiggleAll();
      });
    } catch (e) { setStatus("listen err: " + e); }

    toggleBtn.addEventListener("click", async () => {
      mode = mode === "curt" ? "wordy" : "curt";
      toggleBtn.textContent = mode;
      try { await invoke("set_mode", { mode }); } catch (e) { setStatus("mode err: " + e); }
      wiggleAll();
    });

    document.getElementById("clear").addEventListener("click", async () => {
      const topics = await invoke("clear_topics", { sessionId });
      render(topics);
    });

    chooseScreen();
    return;
  }

  // --- Main window: welcome / greeting / onboarding ---
  document.getElementById("ob-save").addEventListener("click", async () => {
    const name  = document.getElementById("ob-name").value.trim();
    const anth  = document.getElementById("ob-anthropic-key").value.trim();
    const codex = document.getElementById("ob-codex-key").value.trim();
    const err   = document.getElementById("ob-error");
    if (!name) { err.textContent = "name required"; return; }
    if (!anth && !codex) { err.textContent = "one api key required"; return; }
    const key = anth || codex;
    try {
      await invoke("save_config", {
        apiKey: key,
        name,
        agents: ["claude", "codex"],
        mode: "curt",
      });
      onboarded = true;
      userName  = name;
      mode      = "curt";
      toggleBtn.textContent = mode;
      wiggleAll();
      setTimeout(chooseScreen, 900);
    } catch (e) {
      err.textContent = "save error: " + e;
    }
  });

  try {
    await listen("session-active", (evt) => {
      sessionActive = !!evt.payload;
      chooseScreen();
    });
  } catch (e) { setStatus("listen err: " + e); }

  chooseScreen();
}

init().catch(e => { setStatus("init err: " + e); showScreen("welcome"); });
