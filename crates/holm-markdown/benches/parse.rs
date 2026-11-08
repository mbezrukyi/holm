//! Speed of [`Body::parse`] on a 1 MB note.

use holm_markdown::Body;

fn main() {
    divan::main();
}

/// A part of a note with every element, each block closed.
const CHUNK: &str = "## Plans for *this* week

Some ==big== ideas, a <u>key</u> word and <span style=\"color: #c4553a\">red</span> text.
A [note link](holm:01JC3X0000000000000000000A) and a [file](<attachments/my file.pdf>).
Also `code`, **strong** and ~~old~~ text, with \\*an escape\\*.

> [!note]
> A callout with <mark style=\"background: #fde68a\">amber</mark> text.

- [ ] a task
- [x] a done task
  - nested item

| Name | Size |
|:-----|-----:|
| a    | 1    |

![image](attachments/cat.png)

```rust
fn main() {}
```

";

/// About 1 MB of [`CHUNK`] parts, one after another.
fn one_mb_note() -> String {
    CHUNK.repeat(1_000_000 / CHUNK.len() + 1)
}

#[divan::bench]
fn parse_1_mb(bencher: divan::Bencher<'_, '_>) {
    let text = one_mb_note();
    bencher.bench(|| Body::parse(divan::black_box(&text)));
}

/// `pulldown-cmark` alone on the same note, to show the cost of Holm's own pass.
#[divan::bench]
fn pulldown_cmark_only(bencher: divan::Bencher<'_, '_>) {
    use pulldown_cmark::{Options, Parser};

    let text = one_mb_note();
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_GFM;
    bencher.bench(|| {
        Parser::new_ext(divan::black_box(&text), options)
            .into_offset_iter()
            .count()
    });
}
