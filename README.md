# dooni

<p align="center"><img src="docs/logo.png" alt="dooni" width="120" /></p>

A tiny always-on-top desktop widget that keeps a running memo of your AI coding chat sessions (Claude Code, Codex CLI). Long chats accumulate dozens of subtopics; dooni surfaces a persistent, glanceable running list so you never lose the thread.

<img width="342" height="499" alt="Screenshot 2026-07-07 at 2 49 31 AM" src="https://github.com/user-attachments/assets/3c308493-7374-4e1a-b82c-793fb738645e" />

## What it does

- Pops up in the corner of your screen while you're chatting with `claude` or `codex` in a terminal.
- Watches your local session transcripts and, every few prompts, updates a running memo of what you've talked about.
- The list only ever grows — you can glance at it any time to remember what this session has covered.
- Two modes toggled from the top-right corner:
  - **curt** — short bullet topics (e.g. `▢ Tauri event permissions`)
  - **wordy** — full sentences (e.g. `▢ Helen asked why events weren't received, and the assistant found a missing capability`)
  - Switching modes only affects new entries; existing entries stay as they were.
- Aha moments get a 💡 prefix.

## Install

Requirements: macOS, Node 18+, Rust stable, Xcode command line tools.

```sh
git clone https://github.com/ilovehhhyn/dooni.git
cd dooni
npm install
npx tauri dev
```

Grab an Anthropic API key at https://console.anthropic.com/settings/keys and paste it into the onboarding form on first launch.

To build a distributable `.app`:

```sh
npx tauri build
```

## How to change settings

All settings live in a JSON file:

- macOS: `~/Library/Application Support/dooni/config.json`
- Linux: `~/.config/dooni/config.json`

Edit that file directly, or use `jq` as shown below. **Restart dooni for changes to take effect.**

```sh
CFG=~/Library/Application\ Support/dooni/config.json
```

### How to change default memo mode

```sh
jq '.mode = "wordy"' "$CFG" > "$CFG.tmp" && mv "$CFG.tmp" "$CFG"
# or "curt"
```

You can also just click the toggle in the top-right of the dooni window — no restart needed for that.

### How to update API key

```sh
jq '.api_key = "sk-ant-NEW"' "$CFG" > "$CFG.tmp" && mv "$CFG.tmp" "$CFG"
```

### Reset onboarding

```sh
jq '.onboarded = false' "$CFG" > "$CFG.tmp" && mv "$CFG.tmp" "$CFG"
```

Next launch will show the welcome + onboarding screens again.

## More

See [BUILD_PLAN.md](./BUILD_PLAN.md) for architecture, JSONL parsing rules, prompt design, known limitations, and the full rationale.
