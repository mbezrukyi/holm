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
