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

/// Plain prose: paragraphs of many lines with few marks.
const PROSE: &str = "We walked to the old mill by the river and sat on the stone wall for an hour.
The water was low, and the wheel did not turn. A man with a dog came by and told us
the mill closed when he was a boy. He said the miller's family still lives in the town,
in the yellow house next to the bakery, and they keep the key to the door.

On the way back we stopped at the bakery and bought bread for the week.

";

/// Quotes, lists and tables, with a little inline syntax.
const BLOCKS: &str = "> A quote with *one* mark
> that runs over two lines.

- first item
- second item with `code`
  - nested item
1. one
2. two

| Name | Size | Note |
|:-----|-----:|------|
| a    | 1    | **big** |
| b    | 2    | small |

";

/// About 1 MB of `part`, one after another.
fn one_mb(part: &str) -> String {
    part.repeat(1_000_000 / part.len() + 1)
}

/// About 1 MB of [`CHUNK`] parts, one after another.
fn one_mb_note() -> String {
    one_mb(CHUNK)
}

#[divan::bench]
fn parse_1_mb(bencher: divan::Bencher<'_, '_>) {
    let text = one_mb_note();
    bencher.bench(|| Body::parse(divan::black_box(&text)));
}

#[divan::bench]
fn parse_1_mb_prose(bencher: divan::Bencher<'_, '_>) {
    let text = one_mb(PROSE);
    bencher.bench(|| Body::parse(divan::black_box(&text)));
}

#[divan::bench]
fn parse_1_mb_blocks(bencher: divan::Bencher<'_, '_>) {
    let text = one_mb(BLOCKS);
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

/// One char typed in the middle of the note and removed again: two calls of [`Body::reparse`].
#[divan::bench]
fn reparse_after_one_key(bencher: divan::Bencher<'_, '_>) {
    let mut text = one_mb_note();
    let mut body = Body::parse(&text);
    let at = text.floor_char_boundary(text.len() / 2);
    bencher.bench_local(|| {
        text.insert(at, 'a');
        body.reparse(divan::black_box(&text), at..at, 1);
        text.remove(at);
        body.reparse(divan::black_box(&text), at..at + 1, 0);
    });
}
