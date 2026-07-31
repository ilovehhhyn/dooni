# dooni

a small desktop companion for tracking your Codex and Claude coding chats, keeping a running memo of each session (Claude Code, Codex CLI). 

long chats accumulate twists and turns that could fleet your mind; but don't fret! 

dooni keeps a persistent, glanceable running list so you never lose the thread.

## What it does

- After signup, dooni shows one chat list.
- Each launch begins with an empty chat list. A Codex, Claude, or terminal chat is admitted only after its first new substantive user turn after dooni launches.
- The list retains at most the 20 newest admitted chats. Older entries and their windows are removed.
- On first admission, the selected runtime is asked what the chat is about, reads bounded chat content, and generates a descriptive title. dooni prefixes a repository or folder name as `project · title` when available. Titles are reconsidered after every five substantive user prompts unless you rename a chat manually, which locks its title.
- Notes windows open only from the open icon, and they are not pinned by default.
- Each note has two tabs:
  - **Future prompts**: the first and default tab. These are prompts you write yourself. They are editable and checkable. Enter adds a prompt, and Command/Control+Enter inserts a newline.
  - **Asked**: the second tab. It contains your source prompts after removing generated file-attachment scaffolding, internal context, and continuation-only messages.
- AI is used only to generate titles and to classify continuation-only prompts.
- The interface is white monochrome. The blob appears during signup only, with no castle and no post-signup branding.

## Keyboard shortcut

Command-Shift-D is a global macOS shortcut. From Codex, Claude, or a terminal, it opens or focuses the most recently active surfaced dooni window for that surface. Terminal matching is best effort. The shortcut is also documented in Settings.

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

There is no memo mode to change. The curt and default memo modes were removed, along with the mode toggle. Asked now always shows your source prompts after removing generated file-attachment scaffolding, internal context, and continuation-only messages.

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
