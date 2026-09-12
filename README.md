<p align="center">
  <img src="docs/images/banner/banner.png" alt="Wizard101 Chat Translator" width="1000">
</p>

# Wizard101 Chat Translator

English | [繁體中文](README_ZH-TW.md) | [简体中文](README_ZH-CN.md)

[![ci](https://github.com/GoneTone/wizard101-chat-translator/actions/workflows/ci.yml/badge.svg)](https://github.com/GoneTone/wizard101-chat-translator/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/GoneTone/wizard101-chat-translator)](https://github.com/GoneTone/wizard101-chat-translator/releases/latest)
[![Crowdin](https://badges.crowdin.net/wizard101-chat-translator/localized.svg)](https://crowdin.com/project/wizard101-chat-translator)

Wizard101 Chat Translator — an AI translator for Wizard101's in-game chat, translating conversations both ways in real time.

**Reading chat**: every line that appears in chat is translated into the language you set and shown in an overlay window, with the original kept above the translation, so you always see what was actually said.

**Speaking**: open the chat input box in the game and the translation input box comes up on its own. Type in any language and press Enter, and the English translation is typed into the game's chat box character by character (the game does not accept pasted text). The translated line is **never sent for you** — you get to check it, then press Enter to send.

The language chat is translated into is up to you: write the language name straight into the settings (`繁體中文（台灣）`, `日本語`, `Español`, …). The language other people write in is detected by the AI automatically.

Chat is read straight out of the game's chat window rather than off the screen: nothing is misread, nothing arrives out of order, everyone's lines are covered — yours included, along with who said them — and no network traffic is touched.

Posts:

- 巴哈姆特 (Bahamut): <https://forum.gamer.com.tw/C.php?bsn=17541&snA=288&tnum=1>
- 旋風之音 GoneTone - Website：<https://blog.reh.tw/archives/4224>

## ⚠ Important

This software injects into the game process to read in-game conversations. KingsIsle does not sanction this, and it may be treated as a breach of the [Wizard101 Terms of Use](https://www.wizard101.com/game/termsofuse); under those terms KingsIsle may suspend an account for any reason, or for no reason. **Use at your own discretion.**

## Localization

Please help us translate this software!

<https://crowdin.com/project/wizard101-chat-translator>

## Download

Download `Wizard101ChatTranslator.exe` and put it in any folder. The settings file `config.json` and the logs `app.log`, `messages.log` are all created next to the exe.

<https://github.com/GoneTone/wizard101-chat-translator/releases/latest>

**Updating**: close the software first — click the ✕ at the top-right of the overlay's title bar — and only then swap the file. While it is running the exe is locked by Windows and it is still hooked into the game. Once it is closed, download the new `Wizard101ChatTranslator.exe` and overwrite the old one in the same folder; your existing `config.json` is used as it is, so the provider, API key and other settings never have to be entered again.

**Anti-virus false positives**: this software reads the game's chat memory (by hooking into the game process) and listens for a global hotkey. That behavior pattern resembles some malware, so anti-virus programs may flag it or delete/quarantine it outright. This is a false positive; assess the risk yourself and add the program to your anti-virus allowlist.

**Administrator privileges**: normally not required (when the game starts with normal privileges). Only when Wizard101 itself runs **as administrator** must this software also run as administrator (hooking into a process requires equal or higher privileges). If privileges are insufficient, the overlay shows an "⚠  Access denied" banner: close the software, right-click it, choose *Run as administrator*, and start it again.

## How to Use

1. Double-click the exe:
   - When the **translation provider is not set up yet** (first launch, or `config.json` is missing the model, the API key or the self-hosted server URL) a **setup wizard** appears: step 1 picks the UI language, step 2 picks the translation provider, fills in the API key and tests the connection, step 3 picks the target language, the hotkey and whether the translation input box pops up automatically. Once saved, the main flow starts automatically.
   - Once the provider is set up, every later launch goes straight to the main flow — no wizard. The UI language can still be changed at any time in the settings window (gear ⚙); saving applies it immediately, with no restart.
2. Start Wizard101 and **log in, all the way into the game world** — only then can the overlay read chat.
3. A chat message appears → the overlay shows **the original plus the translation** in your target language (newest at the bottom, scroll up for history; by default nothing fades away on a timer, and at most 200 messages are kept with the oldest dropped beyond that; both can be changed on the *Advanced* tab of the settings).
   - **Movable and resizable**: drag the title bar at the top to move it, drag any edge or corner to resize (there is also a grip in the bottom-right corner; the ⚙ / ─ / ✕ buttons on the title bar stay buttons and do not start a resize). Position and size are saved automatically.
   - **Selectable and copyable**: press and drag the left mouse button over the messages to select text, across several messages if you want; dragging past the top or bottom edge of the message area auto-scrolls, so you can reach content that is off-screen. Then press `Ctrl+C`, or right-click the selection and choose *Copy*. Copied text has **a blank line between messages**.
   - Note: the window is not click-through — clicks on the area it covers do not reach the game.
4. To say something: open the chat input box in the game → the translation input box appears on its own, right below the game's chat box and just as wide (re-aligned every time the chat box opens) → type your message (in any language) → Enter → the software switches back to the game and **types the translated English into the chat box character by character** → check it yourself, then press Enter to send.
   - If you closed the translation input box, or turned the automatic popup (`auto_show_input`) off in the settings, press `Ctrl+Space` (the default) to bring it up; it opens under the game's chat box when that is open, otherwise at the mouse cursor. The hotkey only fires while the game window is in the foreground, so it never triggers in other apps.
   - A translation longer than the game chat box's 80-character limit is not typed: the translation input box shows the count and keeps your text so you can trim it or split it up and resend.
   - If the game closes its chat box while you are still typing, the translation input box hides with it but keeps the unsent text and restores it the next time it opens; only closing it yourself with Esc or ✕ discards the text.
   - Characters dropped, or typed too fast, while the translation goes into the game → open the settings window (⚙) and raise *Typing delay (s)* on the *Advanced* tab.
   - Keep the cursor in the game's chat input box while it types — do not click another window.
5. To change settings (provider, API key, target language, hotkey, …), click the gear (⚙) on the overlay's title bar to open the settings window; saving applies immediately, with no restart (the only exception is *Game path* on the *Advanced* tab, which takes effect on the next start).
6. To quit, click the ✕ at the top-right of the overlay's title bar (it unhooks from the game before exiting).
7. Every launch checks GitHub for a new version; if there is one, a blue banner is added to the overlay (click the banner to open the download page, click the ✕ on its right to dismiss it for this session). The *About* tab of the settings window also offers a manual update check, project links, and the folder where the logs live.

## Features

- **Understand what people are saying**: every line of chat is translated into your language in real time, with the original kept above it; it keeps up even when messages flood in
- **Speak in your own language**: open the game's chat input box and the translation input box appears on its own; type, press Enter, and the English translation is typed into the chat box — whether to send it is up to you
- **No misread characters, no missed messages**: the game's chat content is read directly rather than recognized off the screen, and both other people's lines and your own are covered
- **You choose the language to translate into**: write the language name yourself (`繁體中文（台灣）`, `日本語`, `Español`, …); the language other people use is detected by the AI automatically
- **Pick the AI you want**: OpenAI (ChatGPT), Anthropic (Claude), or your own OpenAI-compatible service; each keeps its own settings, so switching back and forth never makes you retype a key
- **System messages, if you want them**: loot, XP, level-up broadcasts and the like are not translated by default — turn them on in the settings when you need them
- **Arrange the window once and forget it**: drag to move, resize from an edge or corner, dial the opacity down, shrink it to a small bubble when you are not reading it; position and size are remembered and it comes back in the same place next time
- **Copy anything worth keeping**: drag over the messages to select text, across several messages, then `Ctrl+C` or right-click to copy
- **Settings can be changed at any time**: a wizard walks you through the first launch; after that click the gear (⚙), and saving takes effect immediately with no restart
- **Survives a forced kill or a crash**: a normal exit unhooks itself; if it is force-killed, the next launch clears the leftovers before hooking in again, with no need to restart the game
- **Multi-language UI, automatic update check**: the first launch picks the UI language from your Windows system language; new versions are announced on the overlay

## Troubleshooting

- **The "Access denied" banner**: the game is running as administrator, so the software cannot hook into it. Close the software and start it as administrator (from source, open the terminal as administrator).
- **The "Game version not supported" banner**: the software finds the chat control through fixed memory patterns, and a game update can invalidate them. First update **both the game and this software** to the latest version; if both are already up to date and the banner is still there, the software has not caught up with this game version yet and you have to wait for a new release (the *About* tab of the settings window has an update check). Nothing is written into the game in this state, so leaving it be has no side effects.
- **Status stuck at "Connecting to game…", or "Game not ready or disconnected"**: make sure the game is running and that you are logged in, all the way into the game world.
- **Characters dropped while the translation is typed in**: raise *Typing delay (s)* on the *Advanced* tab of the settings window (⚙), and make sure the cursor stays in the game's chat input box while it types.

## Screenshots

The overlay over the running game — every chat line keeps its original above the translation:

![The overlay showing a translated conversation over the running game](docs/images/1.png)

Open the game's chat box and the translation input box comes up on its own; type in any language:

![The translation input box with a message typed into it](docs/images/2.png)

The English translation is typed into the game's chat box character by character — pressing Enter to send is up to you:

![The game's chat box with the English translation typed into it](docs/images/3.png)

The settings window: provider, API key, target language and hotkey, all applied without a restart:

![The settings window on its Basic tab](docs/images/4.png)

## Known Limitations

- The game's chat whitelist: English words that are not on the whitelist may be filtered by the game itself, which no translation software can get around.
- Translations do not appear instantly: chat is scanned every 0.4 seconds by default (adjustable on the *Advanced* tab), and a line is only shown once the translation service answers — how long that takes depends mostly on the provider and model you picked.
- Player chat is never cached; every line is really sent for translation (so every line costs an API call). Two reasons: players hardly ever repeat themselves word for word, so a cache would almost never hit; and every translation carries the preceding lines as context, which means the same sentence can call for a different translation in a different conversation — forcing an old one on it would get it wrong. Only system messages, whose phrasing is fixed, are cached.
- It depends on fixed memory patterns: a game update can invalidate them (hooking reports `PatternFailed`, or the hook installs but never fires) and it only works again once support catches up; from source, that means re-running `uv sync` after the reader dependency has been updated. Both cases show the "⚠  Game version not supported" banner in the overlay, which is distinct from "the game is not running".
- Hooking into the game and the global hotkey (the `keyboard` package) only need the same elevation when the game itself runs as administrator (integrity levels have to match); otherwise nothing special is required. When privileges are insufficient the banner spells out the fix.

## Development

The rest of this file covers working from source: development, debugging and packaging.

### Prerequisites

- Windows
- [uv](https://docs.astral.sh/uv/) — the Python version is pinned by `.python-version`, and `uv sync` downloads a matching interpreter, so local and CI environments agree

### Install

1. `uv sync` — creates `.venv` and installs everything in `pyproject.toml` (including wizwalker for the incoming side; `[tool.uv.sources]` already points at the [LaurenzNotHere fork](https://codeberg.org/LaurenzNotHere/wizwalker), which keeps up with the current client patterns, because the official PyPI release has stale patterns that no longer match the live client).
2. `uv run run.py` — the first run goes through the setup wizard (pick a provider, enter an API key or your self-hosted server URL, test the connection); the settings are written to `config.json` in the project root, which is not version-controlled.

> When a game update makes hooking fail with `PatternFailed`, update the fork and re-run `uv sync` (or `uv lock --upgrade-package wizwalker`) to pick up the new patterns.

### Running from source

1. Start Wizard101 and log in, all the way into the game world.
2. `uv run run.py` (equivalent to `uv run python -m src.main`).
3. It works exactly as described in [How to Use](#how-to-use); when a banner shows up, see [Troubleshooting](#troubleshooting).

### Checks

```
uv run ruff check src tests   # lint: unused imports, undefined names, import order (rules in pyproject.toml)
uv run pytest                 # unit tests (4 parallel workers by default; add -n 0 to run serially)
```

Once pushed to GitHub, CI (`.github/workflows/ci.yml`) runs the same lint and tests on a Windows runner.

### Packaging (exe)

```
uv run pyinstaller build.spec --noconfirm
```

The result is `dist/Wizard101ChatTranslator.exe`, a single windowed exe with no console window. That exe is all you need to distribute — `config.json`, `app.log` and `messages.log` are created next to it on the user's first run (see [Download](#download)).

### Logs

Both logs are split per launch, keep the last 7 days only, and start every line with a UTC+0 timestamp:

- `app.log`: diagnostic output (hooking, translation requests, settings changes, exceptions, …).
- `messages.log`: the **raw** chat the reader saw, with no cleanup or filtering (`RAW` is the original line including markup and system messages, excluding only the `[WARN]` / `[ERRO]` / `[DBGM]`-style debug lines the game itself pushes into the chat control; `OUT` is the line actually sent for translation). Attach this one for missed or duplicated translations.

## Icon

The program icon (`src/assets/icon.ico`) is adapted from the Wizard101 game icon with a "文A" translation badge in the top-right corner, shared by the exe and every window. The base artwork is copyright KingsIsle Entertainment; this project is not affiliated with them.
