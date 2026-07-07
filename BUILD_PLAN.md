# dooni — Build Plan

> A pop-up desktop companion that keeps a running memo of topics from your AI-agent chat sessions, so long conversations never lose the thread.

---

## 1. What dooni is

**dooni** is a small always-on-top desktop widget styled like a terminal window. Whenever the user is chatting with an AI coding agent — specifically **Claude**, **Claude Code**, or **Codex CLI** — dooni appears in the corner of the screen and maintains a live, evolving list of topics that have been discussed in the current session.

The purpose is simple: **in long chat sessions, users forget what has already been covered.** dooni is an ambient, glanceable memory aid — a running memo / todo-list of the conversation that updates itself as the chat progresses.

Visual identity:
- Small floating window (~340 × 500).
- Pixelated blob mascot at the top (the dooni face).
- Beneath it: a scrolling running list, styled like a terminal memo / todo list.
- Color scheme: **white background, black text, monospace font** (kept intentionally minimal — one background color and one foreground color).

---

## 2. The core problem being solved

Long chat sessions with AI agents accumulate dozens of subtopics: bug fixes, refactors, questions asked, decisions made, files touched. Users lose track. Scrolling back is expensive. dooni surfaces a persistent, incrementally-updated **table of contents** for the live session, without the user needing to ask.

---

## 3. User journey

1. User installs dooni once.
2. Any time the user opens a terminal and starts chatting with `claude` (Claude Code) or `codex` (Codex CLI), dooni's floating window pops up automatically.
3. The pixel-blob mascot sits at the top of the window. A memo-style list sits below.
4. As the chat progresses, new topics get added to the list; related items get merged; stale phrasing gets rewritten.
5. When the user closes the CLI, dooni fades to a dormant state (or hides).
6. At any time, the user can glance at the window and see "here's everything we've touched this session."

---

## 4. How dooni gets access to the chat (the key architectural question)

Several access strategies were considered:

| # | Approach | Coverage | Effort | Tradeoffs |
|---|----------|----------|--------|-----------|
| 1 | **API proxy** (redirect `ANTHROPIC_BASE_URL` through a local proxy) | Claude Code + Codex + any SDK | Medium | Requires user setup; doesn't cover claude.ai web |
| 2 | **Log file tailing** — read `~/.claude/projects/**/*.jsonl` and `~/.codex/sessions/**/*.jsonl` | Claude Code + Codex CLI | **Low** | No setup; skips claude.ai web |
| 3 | **Claude Code hooks** (`UserPromptSubmit`, `Stop`) | Claude Code only | Low | Doesn't cover Codex |
| 4 | **Browser extension** (DOM / fetch intercept) | claude.ai / ChatGPT web | Medium | Separate integration; not needed for CLI story |
| 5 | **Screen OCR / accessibility APIs** | Universal | High | Fragile, slow, privacy-heavy |

**Chosen approach: #2 — log file tailing.**

Reasons:
- No user setup beyond installing dooni.
- Both Claude Code and Codex CLI already write structured JSONL session transcripts to well-known local paths.
- Zero risk of interfering with the user's actual chat.
- Clean structured data (already parsed by the agents themselves).

Paths watched:
- **Claude Code:** `~/.claude/projects/**/*.jsonl` — each line is a JSON object with `type: "user" | "assistant"` and `message.content` (string or array of content blocks with `text` fields).
- **Codex CLI:** `~/.codex/sessions/**/*.jsonl` (path may vary by version; degrade gracefully if absent).

---

## 5. How the running list gets built

While the log file is being watched, every new user/assistant turn is parsed and appended to an in-memory transcript for that session.

**Trigger cadence:** every **5 user prompts**, dooni calls the Anthropic API to update the topic list. This keeps API usage low ("so there aren't too many burns") while still providing frequent-enough updates in an active session.

**Debounce:** trigger fires 3 seconds after the 5th prompt, so a rapid-fire burst of prompts doesn't cause overlapping API calls.

**Model:** `claude-haiku-4-5-20251001` — fast, cheap, more than smart enough to maintain a short list.

**Prompt design (incremental edit, not full regeneration):**
- The model is given the **current topic list** plus the **last several exchanges**.
- It's asked to return an **updated list** — merging duplicates, renaming for clarity, adding new distinct topics, ordering oldest→newest, max 20 items.
- Output format: strict JSON array of short strings (≤7 words each).
- This "edit in place" approach preserves continuity across summarizations rather than churning the list from scratch every time.

Prompt sketch (system message):

> "You maintain a concise, evolving list of TOPICS being discussed in a live coding chat session. Given the CURRENT LIST and the RECENT MESSAGES, return an UPDATED LIST as a strict JSON array of short strings (each ≤7 words). Rules: merge duplicates, rename for clarity, add new distinct topics from the recent messages, keep prior topics unless clearly superseded, order oldest→newest (newest at the end), max 20 items. Output ONLY the JSON array — no prose, no code fences."

---

## 6. Design choices confirmed by the user

The 7 decisions from planning:

1. **Desktop shell:** Tauri (v2) — lightweight Rust + system webview, ~5 MB vs Electron's ~100 MB, well-suited to a floating always-on-top widget.
2. **When the window appears:** **(b)** watch for `claude` / `codex` processes and show the window only when one is active. (Not always-on-login, not manual-only.)
3. **Log sources:** Claude Code (`~/.claude/projects/**/*.jsonl`) and Codex CLI (`~/.codex/sessions/**/*.jsonl`). Raw Claude API SDK usage is skipped (no log to tail).
4. **Summarizer API:** use the user's existing `ANTHROPIC_API_KEY` env var; call **Claude Haiku 4.5** (`claude-haiku-4-5-20251001`).
5. **Summary trigger:** every 5 user prompts, with a 3-second debounce so rapid activity doesn't spam the API.
6. **"Global changes in mind":** pass the current topic list + last N exchanges each call; ask the model to **edit** the list (add/merge/rename) rather than regenerate.
7. **Visual:** pixel-blob mascot on top, terminal-styled window below. Colors: **white background, black text** only. Monospace font.

---

## 7. Architecture

```
┌──────────────────────────── dooni (Tauri app) ────────────────────────────┐
│                                                                            │
│  Rust backend (src-tauri/)                                                 │
│  ┌────────────────┐  ┌────────────────┐  ┌───────────────────────────┐    │
│  │ process_watch  │  │  watcher       │  │  summarizer               │    │
│  │ (poll ps)      │  │  (tail JSONL)  │  │  (Anthropic API — Haiku)  │    │
│  │ show/hide win  │  │  parse turns   │  │  merges topic list        │    │
│  └───────┬────────┘  └───────┬────────┘  └────────────┬──────────────┘    │
│          │                    │  every 5 user turns    │                   │
│          │                    └───────────► trigger ───┘                   │
│          │                                             │                   │
│          │                     app state (topics)      │                   │
│          │                                             ▼                   │
│          │                              Tauri event: topics-updated        │
│          ▼                                             │                   │
│  ┌────────────────────────────────────────────────────┴─────────────┐     │
│  │ Frontend (src/) — HTML/CSS/JS                                    │     │
│  │  ┌──────────┐                                                    │     │
│  │  │  BLOB    │  ← pixel-art via CSS box-shadows                   │     │
│  │  └──────────┘                                                    │     │
│  │  ▢ topic one                                                     │     │
│  │  ▢ topic two                                                     │     │
│  │  ▢ topic three (newest at bottom)                                │     │
│  │  [clear]                                    updated 12:04:21     │     │
│  └───────────────────────────────────────────────────────────────────┘     │
└────────────────────────────────────────────────────────────────────────────┘
        ▲                                              ▲
        │ reads (read-only)                            │ reads env
        │                                              │
   ~/.claude/projects/**/*.jsonl                ANTHROPIC_API_KEY
   ~/.codex/sessions/**/*.jsonl
```

---

## 8. File layout

```
dooni/
├── package.json                    # tauri CLI dep
├── BUILD_PLAN.md                   # this file
├── src/                            # frontend (loaded as frontendDist)
│   ├── index.html                  # markup: header, blob, list, footer
│   ├── styles.css                  # black/white terminal styling
│   └── app.js                      # Tauri IPC: get_topics, listen "topics-updated"
└── src-tauri/                      # Rust backend
    ├── Cargo.toml                  # deps: tauri, tokio, notify, reqwest, sysinfo, walkdir…
    ├── build.rs
    ├── tauri.conf.json             # window: 340×500, alwaysOnTop, frameless-optional
    ├── icons/icon.png              # placeholder icon
    └── src/
        ├── main.rs                 # app entry, state, IPC commands, spawn tasks
        ├── watcher.rs              # jsonl tailing, per-session state, trigger logic
        ├── process_watch.rs        # poll sysinfo for claude/codex procs, show/hide window
        └── summarizer.rs           # Anthropic API client + prompt + JSON-array parse
```

---

## 9. Backend components

### 9.1 `main.rs`
- Initializes shared `AppState` (per-session state map + global topic list).
- Registers two Tauri commands the frontend can call:
  - `get_topics()` → current list.
  - `clear_topics()` → wipe list (manual reset button in UI).
- Spawns two long-lived async tasks:
  - `watcher::run` — tails JSONL logs, triggers summarizer.
  - `process_watch::run` — polls processes, shows/hides the window.

### 9.2 `process_watch.rs`
- Every 3 s, refresh `sysinfo::System` and scan processes.
- Look for names matching `claude`, `codex`, or paths ending in `/claude` or `/codex`.
- If any active → ensure window is visible. If none → hide (or, for MVP, keep visible so the user can still read the memo).

### 9.3 `watcher.rs`
- Every 2 s, walk `~/.claude/projects` and `~/.codex/sessions` for `*.jsonl` files.
- Pick the **most recently modified** file — that's the current session's transcript.
- For each session, track a `last_processed_offset` (byte offset) so we only read newly appended lines.
- Parse each new line:
  - Claude Code: `{ type: "user"|"assistant", message: { content: string | [{text}] } }`
  - Codex: best-effort `{ role: "user"|"assistant", content: … }`
- Append parsed turns to session's in-memory transcript.
- Count new **user** turns. When `user_count_since_summary ≥ 5`:
  - Sleep 3 s (debounce).
  - Grab the last ~10 turns + current global topic list.
  - Call `summarizer::update_topics`.
  - On success: update global topic list, emit `topics-updated` event to the frontend.

### 9.4 `summarizer.rs`
- Reads `ANTHROPIC_API_KEY` from env.
- POSTs to `https://api.anthropic.com/v1/messages` with:
  - `model: claude-haiku-4-5-20251001`
  - `max_tokens: 512`
  - `system`: the "maintain a topics list" prompt (see §5).
  - `user`: `CURRENT LIST:\n…\n\nRECENT MESSAGES:\n…`
- Parses the response's first text block, strips optional code fences, `serde_json::from_str::<Vec<String>>` to get the new list.
- Caps to 20 items.

---

## 10. Frontend components

### 10.1 `index.html`
- `<header>`: blob div + "dooni" title + "running memo" subtitle.
- `<main>`: `<ul id="topics">` — the memo list.
- `<footer>`: `[clear]` button + status text.

### 10.2 `styles.css`
- Body: white bg, black text, monospace, 13 px.
- Border: 1 px solid black around whole app + dashed dividers between header/main/footer.
- **Pixel blob**: rendered as a single CSS pseudo-element with a large `box-shadow` chain drawing a 7×7-ish pixel grid — no image asset needed, stays crisp at any DPR.
- List items: `▢` bullet, dotted 1-px separator between items, newest at the bottom.
- Button: black border, inverts on hover.

### 10.3 `app.js`
- On load: `invoke("get_topics")` → render.
- `listen("topics-updated", …)` → re-render + update status timestamp.
- Clear button → `invoke("clear_topics")`.

---

## 11. Configuration

- **Env var required at launch:** `ANTHROPIC_API_KEY`. If missing, dooni still runs; the log tailer still watches and displays turns count; the summarizer just logs an error each trigger.
- **Window config** (`tauri.conf.json`): `alwaysOnTop: true`, `width: 340`, `height: 500`, resizable, decorations on (for MVP; can go frameless later).

---

## 12. Testing & debugging strategy

### 12.1 Static build check
- `cargo build` under `src-tauri/` to surface compile errors early.
- `cargo tauri dev` (via `npx @tauri-apps/cli`) to launch the app.

### 12.2 Fixture-driven test
- Create a fake session file at `~/.claude/projects/dooni-test/session-fake.jsonl`.
- Append synthetic user/assistant lines matching the Claude Code JSONL format at a controlled rate.
- Confirm:
  - The tailer picks up new lines within 2 s.
  - After 5 synthetic user prompts, the summarizer fires.
  - Topics list appears in the UI.

### 12.3 Live smoke test
- With `ANTHROPIC_API_KEY` exported, start `cargo tauri dev`, then run `claude` in a terminal in another repo.
- Chat with Claude Code about several distinct subjects (e.g., "explain X", then "help me fix bug Y", then "refactor Z").
- Expect topics like "explain X", "fix bug Y", "refactor Z" to appear.

### 12.4 Common failure modes to check
- **JSONL parsing edge cases:** tool-use content blocks, meta lines (permission-mode, file-history-snapshot). Filter to `type: user | assistant` only; require non-empty text.
- **File truncation/rotation:** if a file's size shrinks below `last_processed_offset`, reset offset to 0 for that session.
- **Multiple concurrent sessions:** tailer picks only the most recently modified file → this is a known MVP limitation. Could be extended to track all sessions active in the last N minutes.
- **API rate/errors:** surface in stderr; UI status line shows "updated <time>" only on success.
- **Missing API key:** log once, don't spam.

### 12.5 Debug affordances
- `eprintln!("[dooni] …")` sprinkled through backend for stderr visibility during `tauri dev`.
- The `[clear]` button in the UI resets the list — useful when iterating on the prompt.

---

## 13. Known non-goals for MVP

- **claude.ai web chat coverage** (would need a browser extension).
- **Multi-session simultaneous tracking** (MVP focuses on the most recently active file).
- **Persistence across dooni restarts** (topics live in memory only).
- **Configurable model / trigger threshold via UI** (hard-coded for now).
- **Tray icon menu** (window is the whole UX for MVP).

---

## 14. Verification checklist

- [ ] `cargo build` succeeds.
- [ ] `npx tauri dev` launches the window.
- [ ] Window shows blob + empty-state message.
- [ ] Synthetic JSONL append triggers a summarizer call after 5 user lines.
- [ ] Real `claude` session updates topics.
- [ ] `[clear]` button empties the list.
- [ ] Closing the CLI eventually stops updates (no crashes).
- [ ] With `ANTHROPIC_API_KEY` unset, app runs without panicking.

---

# Appendix — Concrete Artifacts

Everything below is meant to be **copy-pasteable** so a fresh coding agent can reproduce dooni without guessing.

## A. Prerequisites (host machine)

- **macOS** (primary target; Linux/Windows should work but untested).
- **Node.js** ≥ 18 (for the Tauri CLI).
- **Rust** stable toolchain. If missing:
  ```sh
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal
  source "$HOME/.cargo/env"
  ```
- **Xcode Command Line Tools** (`xcode-select --install`) — required by Tauri on macOS.
- **`ANTHROPIC_API_KEY`** exported in the shell that launches dooni.

## B. Bootstrap commands (from repo root)

```sh
# 1. install tauri CLI
npm install

# 2. dev run (opens the window, hot-reloads frontend)
npx tauri dev

# 3. production bundle (.app / .dmg on macOS)
npx tauri build
```

## C. Exact `package.json`

```json
{
  "name": "dooni",
  "private": true,
  "version": "0.1.0",
  "scripts": { "tauri": "tauri" },
  "devDependencies": { "@tauri-apps/cli": "^2" }
}
```

## D. Exact `src-tauri/Cargo.toml`

```toml
[package]
name = "dooni"
version = "0.1.0"
edition = "2021"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-shell = "2"
tokio = { version = "1", features = ["full"] }
notify = "6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
reqwest = { version = "0.12", features = ["json", "rustls-tls"], default-features = false }
dirs = "5"
sysinfo = "0.31"
walkdir = "2"
anyhow = "1"

[features]
default = ["custom-protocol"]
custom-protocol = ["tauri/custom-protocol"]
```

## E. Exact `src-tauri/tauri.conf.json`

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "dooni",
  "version": "0.1.0",
  "identifier": "com.dooni.app",
  "build": { "frontendDist": "../src" },
  "app": {
    "windows": [{
      "label": "main",
      "title": "dooni",
      "width": 340,
      "height": 500,
      "resizable": true,
      "alwaysOnTop": true,
      "decorations": true,
      "transparent": false,
      "skipTaskbar": false,
      "visible": true
    }],
    "security": { "csp": null }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/icon.png"]
  }
}
```

**Icon requirement:** Tauri requires `src-tauri/icons/icon.png` to exist even in dev. A 32×32 solid-color PNG is fine as a placeholder. Generate one:
```sh
python3 -c "
import struct, zlib
def png(w,h,rgba):
    def c(t,d): return struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d))
    sig=b'\x89PNG\r\n\x1a\n'
    ihdr=struct.pack('>IIBBBBB',w,h,8,6,0,0,0)
    raw=b''.join(b'\x00'+rgba[y*w*4:(y+1)*w*4] for y in range(h))
    return sig+c(b'IHDR',ihdr)+c(b'IDAT',zlib.compress(raw))+c(b'IEND',b'')
open('src-tauri/icons/icon.png','wb').write(png(32,32,bytes([0,0,0,255])*(32*32)))"
```

## F. JSONL log format — real observed samples

### F.1 Claude Code (`~/.claude/projects/<slug>/*.jsonl`)

Each line is one JSON object. Types include `permission-mode`, `file-history-snapshot`, `user`, `assistant`, and others — **only `user` and `assistant` are conversation turns**; the rest must be ignored.

Real example lines (abbreviated):
```json
{"type":"permission-mode","permissionMode":"default","sessionId":"aff54b9a-..."}
{"type":"file-history-snapshot","messageId":"...","snapshot":{...}}
{"parentUuid":null,"isSidechain":false,"type":"user","message":{"role":"user","content":"hello there"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hi! How can I help?"}]}}
```

`message.content` can be **either** a plain string **or** an array of content blocks. Blocks may be `{type:"text", text:"…"}` (extract), `{type:"tool_use", …}` (skip), or `{type:"tool_result", …}` (skip). Extraction rule:
- If `content` is a string → use it directly.
- If `content` is an array → concatenate all `.text` fields from blocks that have a string `.text`.
- Skip if the resulting text is empty/whitespace.

The **project slug** in the path is the user's cwd with `/` → `-` (e.g. `-Users-helenhui-dooni`). One JSONL file per session; filename is the session UUID.

### F.2 Codex CLI

Location and schema are less stable across versions. Two known variants:
- `~/.codex/sessions/**/rollout-*.jsonl`
- `~/.codex/history.jsonl`

Best-effort parser: look for `{ role: "user" | "assistant", content: … }` at the top level. Extract `content` the same way (string or array of blocks). If neither directory exists, silently skip Codex entirely — don't error.

## G. Rust data types & IPC contract

```rust
// shared, in src-tauri/src/main.rs
pub struct AppState {
    pub sessions: Mutex<HashMap<String, SessionState>>, // key = absolute file path
    pub topics:   Mutex<Vec<String>>,                   // global topic list
}

#[derive(Default, Clone)]
pub struct SessionState {
    pub turns: Vec<Turn>,
    pub user_count_since_summary: usize,
    pub last_processed_offset: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Turn { pub role: String, pub text: String }
```

**IPC commands** (registered via `invoke_handler`):
| Command | Args | Returns | Purpose |
|---|---|---|---|
| `get_topics` | — | `Vec<String>` | Initial render |
| `clear_topics` | — | `Vec<String>` (empty) | Manual reset button |

**Events** (emitted from backend to frontend):
| Event | Payload | Fires when |
|---|---|---|
| `topics-updated` | `Vec<String>` | Summarizer returns a new list |

## H. Anthropic API request — exact shape

```http
POST https://api.anthropic.com/v1/messages
x-api-key: $ANTHROPIC_API_KEY
anthropic-version: 2023-06-01
content-type: application/json
```

Body:
```json
{
  "model": "claude-haiku-4-5-20251001",
  "max_tokens": 512,
  "system": "<SYSTEM PROMPT — see §I>",
  "messages": [
    { "role": "user", "content": "CURRENT LIST:\n1. topic a\n2. topic b\n\nRECENT MESSAGES:\n[user] …\n\n[assistant] …\n\nReturn the updated JSON array now." }
  ]
}
```

Response (relevant fields):
```json
{
  "id": "msg_…",
  "type": "message",
  "role": "assistant",
  "content": [ { "type": "text", "text": "[\"topic a\", \"topic b\", \"topic c\"]" } ],
  "stop_reason": "end_turn",
  "usage": { "input_tokens": 123, "output_tokens": 45 }
}
```

Parse `content[0].text`, strip optional ```` ```json ```` / ```` ``` ```` fences, then `serde_json::from_str::<Vec<String>>`. Cap to 20.

## I. Exact prompts

**System prompt (verbatim):**
```
You maintain a concise, evolving list of TOPICS being discussed in a live coding chat session. Given the CURRENT LIST and the RECENT MESSAGES, return an UPDATED LIST as a strict JSON array of short strings (each ≤7 words). Rules: merge duplicates, rename for clarity, add new distinct topics from the recent messages, keep prior topics unless clearly superseded, order oldest→newest (newest at the end), max 20 items. Output ONLY the JSON array — no prose, no code fences.
```

**User message template:**
```
CURRENT LIST:
{numbered list, or "(empty)"}

RECENT MESSAGES:
{last ~10 turns, each formatted as "[role] text", truncated to 800 chars per turn, separated by blank lines}

Return the updated JSON array now.
```

## J. Trigger logic (pseudo-code)

```
every 2s:
  scan ~/.claude/projects/**/*.jsonl and ~/.codex/sessions/**/*.jsonl
  pick file with latest mtime
  read from last_processed_offset to EOF
  for each new line:
    parse -> Turn { role, text }  (skip meta/tool lines)
    append to session.turns
    if role == "user": user_count_since_summary += 1
  save new offset
  if user_count_since_summary >= 5:
    user_count_since_summary = 0
    sleep 3s        # debounce
    call summarizer(current global topics, last 10 turns of this session)
    on success:
      replace global topics
      emit "topics-updated"
```

**Edge cases the agent must handle:**
- File shrank (rotation/truncation): reset that session's `last_processed_offset` to 0.
- Line isn't valid JSON: skip silently.
- `type` isn't `user` or `assistant`: skip.
- `content` extraction returns empty: skip (don't count as a user turn).
- Anthropic returns non-2xx: log to stderr, don't crash, leave topics unchanged.
- `ANTHROPIC_API_KEY` unset: return an error from summarizer; watcher keeps running.

## K. Process detection details

Use `sysinfo::System::refresh_processes()`. Consider a process "an agent session" if **any** of:
- `proc.name().to_lowercase() == "claude"` or `== "codex"`
- `proc.name().to_lowercase().contains("claude-code")`
- `proc.exe()` path ends with `/claude` or `/codex`

Poll every 3 s. On macOS `sysinfo` needs no special entitlements for process listing of the user's own processes.

**MVP simplification (already in the code):** always keep the window visible; only *show* on process detected, don't *hide* when it disappears. This makes the app feel less flickery. Revisit later.

## L. Frontend contract

`src/index.html` structure:
```html
<div id="app">
  <header>
    <div class="blob"></div>
    <div class="title">dooni</div>
    <div class="sub">running memo</div>
  </header>
  <main><ul id="topics">
    <li class="empty">// no topics yet — start chatting</li>
  </ul></main>
  <footer>
    <button id="clear">clear</button>
    <span id="status">watching…</span>
  </footer>
</div>
```

`src/app.js` — uses global `window.__TAURI__.core.invoke` and `window.__TAURI__.event.listen` (Tauri v2 injects these; **no npm imports needed** because we're using the vanilla-HTML approach, not a bundler):
```js
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

async function render(topics) { /* rebuild <ul> */ }
invoke("get_topics").then(render);
listen("topics-updated", e => render(e.payload));
document.getElementById("clear")
  .addEventListener("click", () => invoke("clear_topics").then(render));
```

## M. Pixel-blob recipe

The blob is drawn with **one** `::before` pseudo-element (a 6×6 px black square) and a `box-shadow` list that clones it onto a pixel grid. Bitmap (1 = black, 0 = white, 7 columns × 7 rows):
```
0 1 1 1 0 0 0
1 1 1 1 1 0 0
1 1 0 1 0 1 1
1 1 1 1 1 1 1
1 1 1 1 1 1 1
0 1 1 1 1 1 0
0 0 1 1 1 0 0
```
Each `1` becomes a `box-shadow: (col*6px) (row*6px) #000`. The exact CSS is already in `src/styles.css` — do not treat the coordinates as arbitrary; they encode the bitmap above.

## N. Manual test recipe (no live CLI needed)

```sh
mkdir -p ~/.claude/projects/dooni-test
FILE=~/.claude/projects/dooni-test/fake-$(date +%s).jsonl
for i in 1 2 3 4 5; do
  printf '%s\n' "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"question number $i about topic $i\"}}" >> "$FILE"
  printf '%s\n' "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"answer $i\"}]}}" >> "$FILE"
  sleep 1
done
```
Expect: within ~5 s of the 5th user line, the summarizer fires and the UI renders a list of ~5 topics.

## O. Directory tree after build

```
dooni/
├── BUILD_PLAN.md
├── package.json
├── package-lock.json
├── node_modules/                 # after npm install
├── src/
│   ├── index.html
│   ├── styles.css
│   └── app.js
└── src-tauri/
    ├── Cargo.toml
    ├── Cargo.lock                # after cargo build
    ├── build.rs
    ├── tauri.conf.json
    ├── icons/icon.png
    ├── target/                   # after cargo build
    └── src/
        ├── main.rs
        ├── watcher.rs
        ├── process_watch.rs
        └── summarizer.rs
```

## P. Gaps a fresh agent might still hit

- **Tauri v2 API surface churn**: if `invoke_handler` / `Emitter::emit` signatures have shifted since this plan was written, check https://v2.tauri.app. The IPC contract (§G) is stable in intent even if the exact method names move.
- **Codex log schema drift**: if the parser in §F.2 finds no matching lines, log a warning and move on — don't block on it.
- **macOS Gatekeeper on `tauri build` output**: unsigned `.app` bundles will require right-click → Open the first time. Not a bug.
- **Frameless-window variant**: if the user later wants no title bar, set `"decorations": false` in `tauri.conf.json` and add a small draggable region in the header (`-webkit-app-region: drag`).

