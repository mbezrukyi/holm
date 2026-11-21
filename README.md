<p align="center">
  <img src="packaging/linux/io.github.mbezrukyi.Holm.svg" width="128" height="128" alt="The Holm icon: a warm hill over the sea">
</p>

<h1 align="center">Holm</h1>

<p align="center">
  A calm, keyboard-first desktop app for Markdown notes on Linux.
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#first-steps">First steps</a> ·
  <a href="#report-a-bug">Report a bug</a>
</p>

---

## What it is

- **Your files.** Notes are plain `.md` files in one folder. Any other tool can read them.
- **Live preview.** Markdown shows as styled text while you type. Its marks show only on the line with the cursor.
- **Keyboard first.** Every action has a key, and `Ctrl+?` lists them all.
- **Linked notes.** Links, backlinks, tags, pins, full-text search and quick open.
- **Quiet.** Holm saves on its own, never loses typed text, and uses no network.

## Install

Holm runs on x86_64 Linux. Each [release](https://github.com/mbezrukyi/holm/releases) has an AppImage and a `SHA256SUMS` file.

1. Download both files and check the AppImage:

   ```sh
   gh release download --repo mbezrukyi/holm --pattern '*.AppImage' --pattern SHA256SUMS
   sha256sum -c SHA256SUMS
   ```

2. Open the AppImage with [Gear Lever](https://flathub.org/apps/it.mijorus.gearlever). It keeps the file in one place and puts Holm in your app menu.

To update, download the new release and open it with Gear Lever again. Holm has no updater.

## First steps

At the first start, Holm asks for your notes folder.

| Key | Action |
| --- | --- |
| `Ctrl+N` | New note |
| `Ctrl+Shift+O` | Open a note by its title |
| `Ctrl+Shift+F` | Search the notes |
| `Ctrl+K` | Link a note |
| `Ctrl+Shift+L` | Show or hide the sidebar |
| `Ctrl+?` | All shortcuts |
| `F10` | App menu and settings |

The settings file is `~/.config/holm/config.toml`.

## Report a bug

Open an [issue](https://github.com/mbezrukyi/holm/issues). Write what you did, what you expected, and what happened.

Attach the log, `~/.local/state/holm/holm.log`. It is the log of today, and the logs of the last 7 days are in the same folder. The log holds paths and note ids, never note text.

- After a crash, Holm shows the log path at the next start.
- For more detail, start Holm with `HOLM_LOG=debug holm` and do the steps again.
