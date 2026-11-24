---
name: profile
description: Profiles Holm's benches and speed tests with perf, with call chains. Use when you need to find where Holm spends time or memory.
---

# Profile

Call chains need `kernel.perf_event_paranoid` at 1 or lower. If it is higher, ask the user to set it.

1. Build the bench with frame pointers and line tables, in its own target folder. The last line names the binary:

   ```sh
   CARGO_TARGET_DIR=target/fp RUSTFLAGS="-C force-frame-pointers=yes" \
     CARGO_PROFILE_BENCH_DEBUG=line-tables-only \
     cargo bench -p holm-editor --bench edit --no-run
   ```

   For a speed test like `window::tests::speed`, use `cargo test --release -p holm --no-run` and `CARGO_PROFILE_RELEASE_DEBUG`.

2. Record one bench. The binary needs `--bench`, or it runs each case only once:

   ```sh
   perf record -o target/perf.data --call-graph fp target/fp/release/deps/edit-<hash> --bench key_press_in_chip_lines
   ```

3. Read the report as text:

   ```sh
   perf report -i target/perf.data --stdio --children -g none
   ```

## Rules

- Use `--call-graph fp`, not `dwarf`. GPUI stacks are deeper than the stack copy of `dwarf`, so most of its chains stop inside GPUI.
- Keep the own target folder. Other `RUSTFLAGS` in `target/` would rebuild everything.
- Write `perf.data` to `target/`. The temp folder has a small quota.
- The workspace forbids `unsafe`. Put a counting allocator in a scratch crate outside the workspace.
