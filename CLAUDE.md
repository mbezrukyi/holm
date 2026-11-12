# Holm

## Profiling

Use `perf` for profiles. Call chains need `kernel.perf_event_paranoid` at 1 or lower.

1. Build the bench with debug info. The last line names the binary:

   ```sh
   CARGO_PROFILE_BENCH_DEBUG=true cargo bench -p holm-markdown --bench parse --no-run
   ```

2. Record one bench, with its name as a filter:

   ```sh
   perf record -o target/perf.data --call-graph dwarf target/release/deps/parse-<hash> parse_1_mb
   ```

3. Read the report as text:

   ```sh
   perf report -i target/perf.data --stdio --children -g none
   ```

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
