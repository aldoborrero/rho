use criterion::{Criterion, criterion_group, criterion_main};
use rho_tui::{
	BoxSymbols, Component, RoundedBoxSymbols, SymbolTheme,
	components::markdown::{Markdown, MarkdownTheme},
};

/// Create a minimal no-op theme that isolates parsing/layout cost from
/// ANSI formatting.
fn noop_theme() -> MarkdownTheme {
	MarkdownTheme {
		heading:           Box::new(|s| s.to_owned()),
		link:              Box::new(|s| s.to_owned()),
		link_url:          Box::new(|s| s.to_owned()),
		code:              Box::new(|s| s.to_owned()),
		code_block:        Box::new(|s| s.to_owned()),
		code_block_border: Box::new(|s| s.to_owned()),
		quote:             Box::new(|s| s.to_owned()),
		quote_border:      Box::new(|s| s.to_owned()),
		hr:                Box::new(|s| s.to_owned()),
		list_bullet:       Box::new(|s| s.to_owned()),
		bold:              Box::new(|s| s.to_owned()),
		italic:            Box::new(|s| s.to_owned()),
		strikethrough:     Box::new(|s| s.to_owned()),
		underline:         Box::new(|s| s.to_owned()),
		highlight_code:    None,
		get_mermaid_image: None,
		symbols:           SymbolTheme {
			cursor:         " ",
			input_cursor:   "|",
			box_round:      RoundedBoxSymbols {
				top_left:     "╭",
				top_right:    "╮",
				bottom_left:  "╰",
				bottom_right: "╯",
				horizontal:   "─",
				vertical:     "│",
			},
			box_sharp:      BoxSymbols {
				top_left:     "┌",
				top_right:    "┐",
				bottom_left:  "└",
				bottom_right: "┘",
				horizontal:   "─",
				vertical:     "│",
				tee_down:     "┬",
				tee_up:       "┴",
				tee_left:     "┤",
				tee_right:    "├",
				cross:        "┼",
			},
			table:          BoxSymbols {
				top_left:     "┌",
				top_right:    "┐",
				bottom_left:  "└",
				bottom_right: "┘",
				horizontal:   "─",
				vertical:     "│",
				tee_down:     "┬",
				tee_up:       "┴",
				tee_left:     "┤",
				tee_right:    "├",
				cross:        "┼",
			},
			quote_border:   "▐",
			hr_char:        "─",
			spinner_frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
		},
		highlight_colors:  None,
	}
}

fn make_markdown(text: &str) -> Markdown {
	Markdown::new(text, 0, 0, noop_theme(), None, 2)
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

fn bench_render(c: &mut Criterion) {
	let mut group = c.benchmark_group("render");

	// Plain paragraph
	let plain = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor \
	             incididunt ut labore et dolore magna aliqua.";

	// Formatted text
	let formatted = "Here is some **bold text** and *italic text* and `inline code`. Also \
	                 **another bold** with *more italic* and `more code`. Final line with **bold** \
	                 and *italic* mixed together.";

	// Code block with Rust syntax
	let code_block = "\
```rust
use std::collections::HashMap;

fn fibonacci(n: u64) -> u64 {
    let mut memo = HashMap::new();
    fib_memo(n, &mut memo)
}

fn fib_memo(n: u64, memo: &mut HashMap<u64, u64>) -> u64 {
    if n <= 1 {
        return n;
    }
    if let Some(&val) = memo.get(&n) {
        return val;
    }
    let result = fib_memo(n - 1, memo) + fib_memo(n - 2, memo);
    memo.insert(n, result);
    result
}

fn main() {
    for i in 0..20 {
        println!(\"fib({i}) = {}\", fibonacci(i));
    }
}
```";

	// Nested list
	let nested_list = "\
- Item 1
  - Sub item 1a
    - Deep item 1a-i
  - Sub item 1b
- Item 2
  - Sub item 2a
    - Deep item 2a-i
  - Sub item 2b
- Item 3
  - Sub item 3a";

	// Table
	let table = "\
| Name     | Type    | Default | Description          |
|----------|---------|---------|----------------------|
| width    | usize   | 80      | Terminal width       |
| height   | usize   | 24      | Terminal height      |
| color    | bool    | true    | Enable color output  |
| verbose  | bool    | false   | Verbose logging      |
| timeout  | u64     | 30      | Timeout in seconds   |";

	// Large mixed document (~5KB)
	let large_doc = format!(
		"\
# Main Title

{plain}

## Code Section

{code_block}

## Lists

{nested_list}

## Formatted

{formatted}

## Table

{table}

## Another Section

{plain}

### Sub-heading

{formatted}

{code_block}

{nested_list}
"
	);

	group.bench_function("plain_paragraph", |b| {
		let mut md = make_markdown(plain);
		b.iter(|| {
			md.set_text(plain);
			md.render(std::hint::black_box(80))
		});
	});

	group.bench_function("formatted_text", |b| {
		let mut md = make_markdown(formatted);
		b.iter(|| {
			md.set_text(formatted);
			md.render(std::hint::black_box(80))
		});
	});

	group.bench_function("code_block_rust", |b| {
		let mut md = make_markdown(code_block);
		b.iter(|| {
			md.set_text(code_block);
			md.render(std::hint::black_box(80))
		});
	});

	group.bench_function("nested_list", |b| {
		let mut md = make_markdown(nested_list);
		b.iter(|| {
			md.set_text(nested_list);
			md.render(std::hint::black_box(80))
		});
	});

	group.bench_function("table", |b| {
		let mut md = make_markdown(table);
		b.iter(|| {
			md.set_text(table);
			md.render(std::hint::black_box(80))
		});
	});

	group.bench_function("large_mixed_doc", |b| {
		let mut md = make_markdown(&large_doc);
		b.iter(|| {
			md.set_text(&large_doc);
			md.render(std::hint::black_box(120))
		});
	});

	group.finish();
}

criterion_group!(benches, bench_render);
criterion_main!(benches);
