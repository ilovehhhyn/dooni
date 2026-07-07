# dooni

A tiny always-on-top desktop widget that keeps a running memo of what you've discussed with your AI coding agent (Claude Code, Codex CLI). Long chat sessions accumulate dozens of subtopics — dooni surfaces a persistent, glanceable running list so you never lose the thread.

## What it does

- Watches `~/.claude/projects/**/*.jsonl` and `~/.codex/sessions/**/*.jsonl` for live session transcripts.
- Every 5 user prompts, sends the recent turns + current memo to Claude Haiku 4.5 with a "grow this list" prompt.
- Renders the memo in a small always-on-top window. The list **only ever grows**.
- Two modes: **curt** (short bullet topics) and **wordy** (full "Helen asked X, and the assistant did Y" sentences). Toggle in the top-right — switching only affects future entries; past ones stay in the style they were recorded in.

## Screens

- **First launch, nothing happening yet** → bare blob + `welcome to dooni`.
- **First launch, live claude/codex session running** → onboarding: name + API key → `start`.
- **Returning user, idle** → friendly greeting ("dooni hopes helen is drinking water").
- **Session activity or existing memo** → the running list.

## Install & run

Requirements: macOS, Node 18+, Rust stable, Xcode command line tools.

```sh
git clone <this repo>
cd dooni
npm install
npx tauri dev
```

Grab an Anthropic API key at https://console.anthropic.com/settings/keys and paste it into the onboarding form on first launch.

To build a distributable `.app`:

```sh
npx tauri build
```

## Configuration

There is **no CLI**. All settings live in a JSON file:

- macOS: `~/Library/Application Support/dooni/config.json`
- Linux: `~/.config/dooni/config.json`

Example:

```json
{
  "api_key": "sk-ant-...",
  "name": "helen",
  "agents": ["claude", "codex"],
  "mode": "curt",
  "onboarded": true
}
```

### Changing things after onboarding

Edit that file directly (dooni reads it on launch), or use `jq`:

```sh
CFG=~/Library/Application\ Support/dooni/config.json

# Update your API key
jq '.api_key = "sk-ant-NEW"' "$CFG" > "$CFG.tmp" && mv "$CFG.tmp" "$CFG"

# Set default mode to wordy
jq '.mode = "wordy"' "$CFG" > "$CFG.tmp" && mv "$CFG.tmp" "$CFG"

# Reset onboarding (see the welcome screen again)
jq '.onboarded = false' "$CFG" > "$CFG.tmp" && mv "$CFG.tmp" "$CFG"
```

Restart dooni for changes to take effect. Environment variable `ANTHROPIC_API_KEY` is used as a fallback if the config file has no key.

## curt vs wordy

- **curt** — `▢ Tauri v2 event permissions`
- **wordy** — `▢ Helen asked why the frontend wasn't receiving events, and the assistant found the capabilities file was missing core:event:allow-listen`

Aha moments in either mode get a 💡 prefix. Switching modes only affects future entries; existing ones stay in the style they were recorded in (no token waste re-summarizing).

## Design notes

See [BUILD_PLAN.md](./BUILD_PLAN.md) for architecture, JSONL parsing rules, prompt design, and the full rationale.

## Known limitations

- Tracks the most recently modified JSONL file only — multi-session concurrent tracking is not implemented.
- claude.ai web chat is not covered (no local log to tail).
- Topics live in memory; restarting dooni clears the memo.
- The `codex` API key input in onboarding is stored but only Anthropic's API is used for summarization today.
- The `agents` field in `config.json` is stored but not yet used to filter which log directories are watched — both are always watched. To fully disable one, delete or rename the corresponding directory (`~/.claude/projects` or `~/.codex/sessions`).
