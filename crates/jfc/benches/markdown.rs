//! Benchmarks for the markdown sanitization / wrapping hot paths.
//!
//! These functions run on every streamed assistant chunk before it is
//! rendered in the TUI, so their throughput directly affects redraw latency.
//! We exercise representative payloads: prose with fenced code blocks, the
//! inline tool-call XML wall that leaks from some providers, and long lines
//! that need cell-width-aware hard wrapping.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use jfc_markdown::{hard_wrap_str, has_unclosed_fence, strip_inline_tool_xml};

/// A realistic mixed-content assistant message: prose plus a couple of fenced
/// code blocks, repeated to roughly emulate a multi-KB streamed turn.
fn sample_markdown() -> String {
    let chunk = "\
Here is a short explanation of the change.

```rust
fn main() {
    let xs: Vec<i32> = (0..100).collect();
    println!(\"{}\", xs.iter().sum::<i32>());
}
```

And some follow-up prose describing why the refactor matters, with a bit of
`inline code` and a list:

- first point that is reasonably long so wrapping has something to chew on
- second point, equally verbose, mentioning ratatui and crossterm in passing
- third point closing things out

~~~text
plain fenced block
~~~
";
    chunk.repeat(8)
}

/// The pathological inline tool-call wall that `strip_inline_tool_xml` exists
/// to collapse.
fn sample_tool_xml() -> String {
    let block = "<tool_call>{\"name\":\"bash\",\"input\":{\"command\":\"ls -la /very/long/path\"}}</tool_call>\
<tool_result>{\"stdout\":\"total 0\\ndrwxr-xr-x  2 user user 4096 Jan  1 00:00 .\"}</tool_result>";
    let mut s = String::from("Some leading prose before the tool wall.\n\n");
    for _ in 0..64 {
        s.push_str(block);
        s.push('\n');
    }
    s.push_str("\nTrailing prose after the wall.");
    s
}

fn bench_has_unclosed_fence(c: &mut Criterion) {
    let md = sample_markdown();
    c.bench_function("has_unclosed_fence", |b| {
        b.iter(|| has_unclosed_fence(black_box(&md)))
    });
}

fn bench_strip_inline_tool_xml(c: &mut Criterion) {
    let xml = sample_tool_xml();
    c.bench_function("strip_inline_tool_xml", |b| {
        b.iter(|| strip_inline_tool_xml(black_box(&xml)))
    });
}

fn bench_hard_wrap_str(c: &mut Criterion) {
    // Mix of ASCII and full-width CJK so the cell-width path is exercised.
    let line =
        "The quick brown fox jumps over the lazy dog 日本語のテキストもここに含まれています "
            .repeat(16);
    c.bench_function("hard_wrap_str/width_80", |b| {
        b.iter(|| hard_wrap_str(black_box(&line), black_box(80)))
    });
}

criterion_group!(
    benches,
    bench_has_unclosed_fence,
    bench_strip_inline_tool_xml,
    bench_hard_wrap_str
);
criterion_main!(benches);
