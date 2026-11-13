# Holm

## Profiling

Use `perf` for profiles. Call chains need `kernel.perf_event_paranoid` at 1 or lower.

1. Build the bench with frame pointers and line tables, in its own target folder. The last line names the binary:

   ```sh
   CARGO_TARGET_DIR=target/fp RUSTFLAGS="-C force-frame-pointers=yes" \
     CARGO_PROFILE_BENCH_DEBUG=line-tables-only \
     cargo bench -p holm-editor --bench edit --no-run
   ```

   For a test like `window_speed`, use `cargo test --release -p holm --no-run` and `CARGO_PROFILE_RELEASE_DEBUG`.

2. Record one bench. The binary needs `--bench`, or it runs each case only once:

   ```sh
   perf record -o target/perf.data --call-graph fp target/fp/release/deps/edit-<hash> --bench key_press_in_chip_lines
   ```

3. Read the report as text:

   ```sh
   perf report -i target/perf.data --stdio --children -g none
   ```

- Use `--call-graph fp`, not `dwarf`. GPUI stacks are deeper than the stack copy of `dwarf`, so most of its chains stop inside GPUI.
- The own target folder keeps the normal build cache. Other `RUSTFLAGS` rebuild everything.
- Write `perf.data` to `target/`. The temp folder has a small quota.
- A profile without call chains hides the caller of inlined code, like `ptr::write`.
- Kernel frames like `asm_exc_page_fault` show time spent on new memory.
- The workspace forbids `unsafe`. A counting allocator goes in a scratch crate outside it.

## Looking at the app

Use the `look` skill. Holm needs a notes folder in a config file.

1. In a temp folder `<tmp>`, make `config/holm`, `state`, `cache` and `notes`.
2. Put the test notes in `<tmp>/notes`.
3. Write `<tmp>/config/holm/config.toml`:

   ```toml
   version = 1
   notes_folder = "<tmp>/notes"
   ```

4. Build with `cargo build -p holm`, then run:

   ```sh
   look.sh run env XDG_CONFIG_HOME=<tmp>/config XDG_STATE_HOME=<tmp>/state XDG_CACHE_HOME=<tmp>/cache target/debug/holm
   ```
