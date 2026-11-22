---
name: look
description: Runs Holm on a hidden display with its own notes folder. Sends keys and clicks to it, and takes screenshots. Use when you need to see a change in Holm's window yourself. It never uses the user's screen, keyboard or mouse.
---

# Look

`scripts/look.sh`, next to this file, starts a hidden Wayland display with sway. A VNC server, wayvnc, shows this display. The script drives it with vncdotool (`vncdo`). Holm on this display never shows on the user's screen. It gets no input from the user's keyboard or mouse.

It needs `sway`, `wayvnc` and `vncdotool`. If one is missing, ask the user to install it.

## Setup

Holm reads its notes folder from a config file. Give it its own folders in a temp folder `<tmp>`, so it never changes the user's notes or settings.

1. Make `<tmp>/config/holm`, `<tmp>/state`, `<tmp>/cache` and `<tmp>/notes`.
2. Put the test notes in `<tmp>/notes`.
3. Write `<tmp>/config/holm/config.toml`:

   ```toml
   version = 1
   notes_folder = "<tmp>/notes"
   ```

4. Build: `cargo build -p holm`.

## Steps

1. Start the display: `look.sh start`. The size is 1200x800. To change it, add a size, for example `look.sh start 800x600`.
2. Run Holm. It waits for the window:

   ```sh
   look.sh run env XDG_CONFIG_HOME=<tmp>/config XDG_STATE_HOME=<tmp>/state XDG_CACHE_HOME=<tmp>/cache target/debug/holm
   ```

3. Drive it: `look.sh do <vncdo commands>`. One call can hold many commands. For example:
   - `type 'some text'` types the text.
   - `key enter`, `key ctrl-end` press keys.
   - `move 300 50 click 1` clicks at x 300, y 50. The point is in pixels, from the top left.
   - `pause 0.3` waits 0.3 seconds.
   - `capture <file.png>` takes a screenshot. Then read the file to see it.
4. Stop: `look.sh stop`.

For example: `look.sh do key ctrl-end key enter pause 0.3 capture <tmp>/after.png`

## Rules

- Put `pause 0.3` before each `capture`. Holm needs time to draw after keys or clicks.
- Put screenshots in the temp folder, not in the repo.
- Always run `stop` at the end, also after a failure.
- Each session needs its own display. Set `LOOK_NAME` to a short name of your own, for example `LOOK_NAME=t047`. The default is `main`.
- Holm's log is `$XDG_RUNTIME_DIR/look-<name>/app.log`. `stop` deletes it.
- To compare two states, take a screenshot of each.
