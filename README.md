# dooni

a tiny always-on-top desktop widget that keeps a running memo of your AI coding chat sessions (Claude Code, Codex CLI). 

long chats accumulate twists and turns that could fleet your mind; but don't fret! 

dooni keeps a persistent, glanceable running list so you never lose the thread.

  <p align="center">                                                                                                     
    <img width="340" height="500" alt="Screenshot 2026-07-07 at 2 50 34 AM"                                              
  src="https://github.com/user-attachments/assets/431cf34d-b371-4cc6-b148-be0d23f4701f" />                               
  </p> 
  
## What it does

- After signup, dooni shows one chat list.
- It tracks the newest 20 Codex and Claude JSONL chats. Older entries and their windows are removed.
- Notes windows open only from the open icon, and they are not pinned by default.
- Each note has two sections:
  - **Asked**: your prompts, mirrored verbatim from the source chat. Internal context messages and continuation-only messages are excluded.
  - **Future prompts**: prompts you write yourself. They are editable and checkable. Enter adds a prompt, and Command/Control+Enter inserts a newline.
- AI is used only to generate titles and to classify continuation-only prompts.
- The interface is white monochrome. The blob appears during signup only, with no castle and no post-signup branding.

## Install

Requirements: macOS, Node 18+, Rust stable, Xcode command line tools.

The backend is Rust, so you need the Rust toolchain. If you don't have it:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"   # or open a new terminal
xcode-select --install       # if the command line tools aren't installed yet
```

Then:

```sh
git clone https://github.com/ilovehhhyn/dooni.git
cd dooni
./dooni install
```

dooni is free. The app is not signed or notarized.

Codex runtime authentication is the default, so you do not need an API key to start. The Anthropic runtime is optional.

## How to change settings

All settings live in a JSON file:

- macOS: `~/Library/Application Support/dooni/config.json`
- Linux: `~/.config/dooni/config.json`

Edit that file directly, or use `jq` as shown below. **Restart dooni for changes to take effect.**

```sh
CFG=~/Library/Application\ Support/dooni/config.json
```

### How to change default memo mode

There is no memo mode to change. The curt and default memo modes were removed, along with the mode toggle. Asked now always mirrors substantive source prompts verbatim, excluding internal context and continuation-only messages.

### How to update API key

An API key is only needed for the optional Anthropic runtime; Codex runtime authentication is the default. Change the key in Settings.

### Reset onboarding

```sh
jq '.onboarded = false' "$CFG" > "$CFG.tmp" && mv "$CFG.tmp" "$CFG"
```

Next launch will show the welcome + onboarding screens again.

## More

See [BUILD_PLAN.md](./BUILD_PLAN.md) for architecture, JSONL parsing rules, prompt design, known limitations, and the full rationale.

<p align="center"><img src="docs/logo.png" alt="dooni" width="120" /></p>
