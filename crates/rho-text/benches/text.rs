use criterion::{Criterion, criterion_group, criterion_main};
use rho_text::{
	EllipsisKind, extract_segments_str, sanitize_text_str, slice_with_width_str,
	truncate_to_width_str, visible_width_str, wrap_text_with_ansi_str,
};

// ---------------------------------------------------------------------------
// visible_width
// ---------------------------------------------------------------------------

fn bench_visible_width(c: &mut Criterion) {
	let mut group = c.benchmark_group("visible_width");

	// --- ASCII ---
	let ascii_short = "hello";
	let ascii_medium = "hello world this is a plain ASCII string with some words";
	let ascii_long = "a".repeat(500);

	group.bench_function("ascii_short", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(ascii_short)));
	});
	group.bench_function("ascii_medium", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(ascii_medium)));
	});
	group.bench_function("ascii_long_500", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(&ascii_long)));
	});

	// --- ANSI escape codes ---
	let ansi_simple = "\x1b[31mred\x1b[0m";
	let ansi_complex = "\x1b[31mred text\x1b[0m and \x1b[4munderlined content\x1b[24m with more \
	                    \x1b[1;33;44mstyles\x1b[0m";
	let ansi_nested = "\x1b[1m\x1b[31m\x1b[4mbold red underline\x1b[0m normal \x1b[32mgreen\x1b[0m";
	let ansi_256 = "\x1b[38;5;196mred text\x1b[0m";

	group.bench_function("ansi_simple", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(ansi_simple)));
	});
	group.bench_function("ansi_complex", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(ansi_complex)));
	});
	group.bench_function("ansi_nested", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(ansi_nested)));
	});
	group.bench_function("ansi_256_color", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(ansi_256)));
	});

	// --- OSC 8 hyperlinks ---
	let link_single = "prefix \x1b]8;;https://example.com\x07link text\x1b]8;;\x07 suffix";
	let link_multiple =
        "Click \x1b]8;;https://a.com\x07here\x1b]8;;\x07 or \x1b]8;;https://b.com\x07there\x1b]8;;\x07 for info";

	group.bench_function("osc8_link_single", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(link_single)));
	});
	group.bench_function("osc8_link_multiple", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(link_multiple)));
	});

	// --- CJK characters ---
	let cjk_short = "日本語";
	let cjk_medium = "日本語のテキストとemoji";
	let cjk_long = "日本語のテキストと中文字符和한국어문자混合在一起形成很长的字符串";

	group.bench_function("cjk_short", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(cjk_short)));
	});
	group.bench_function("cjk_medium", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(cjk_medium)));
	});
	group.bench_function("cjk_long", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(cjk_long)));
	});

	// --- Emoji ---
	let emoji_simple = "👋🌍";
	let emoji_complex =
		"Hello 👨\u{200d}👩\u{200d}👧\u{200d}👦 family! 🚀✨🎉 Let's go! 🇺🇸🏳\u{fe0f}\u{200d}🌈";
	let emoji_zwj = "👨\u{200d}💻👩\u{200d}🔬👨\u{200d}👩\u{200d}👧\u{200d}👦";

	group.bench_function("emoji_simple", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(emoji_simple)));
	});
	group.bench_function("emoji_complex", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(emoji_complex)));
	});
	group.bench_function("emoji_zwj", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(emoji_zwj)));
	});

	// --- Mixed content ---
	let mixed_short = "Hello 世界 🌍";
	let mixed_medium = "\x1b[32mStatus:\x1b[0m 成功 ✓ (took 42ms)";
	let mixed_long = "\x1b[1;34m[INFO]\x1b[0m Processing 日本語テキスト with emoji 🚀 and \x1b]8;;https://example.com\x07links\x1b]8;;\x07 完了";

	group.bench_function("mixed_short", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(mixed_short)));
	});
	group.bench_function("mixed_medium", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(mixed_medium)));
	});
	group.bench_function("mixed_long", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(mixed_long)));
	});

	// --- Edge cases ---
	let tabs = "col1\tcol2\tcol3\tcol4";
	let empty = "";
	let newlines = "line1\nline2\nline3";
	let control_chars = "text\x00with\x01control\x02chars";

	group.bench_function("edge_tabs", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(tabs)));
	});
	group.bench_function("edge_empty", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(empty)));
	});
	group.bench_function("edge_newlines", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(newlines)));
	});
	group.bench_function("edge_control_chars", |b| {
		b.iter(|| visible_width_str(std::hint::black_box(control_chars)));
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// wrap
// ---------------------------------------------------------------------------

fn bench_wrap(c: &mut Criterion) {
	let mut group = c.benchmark_group("wrap");

	let short = "hello world";
	let long_plain = "The quick brown fox jumps over the lazy dog. ".repeat(10);
	let ansi_long =
		format!("{}Normal text and more words here. ", "\x1b[1;34mBold blue\x1b[0m ").repeat(8);
	let cjk_text = "世界你好测试文本这是一段很长的中文句子用来测试换行功能是否正确。".repeat(3);
	let wrapped_tabs = "This is a long line that should wrap multiple times when rendered with \
	                    ANSI \x1b[32mcolors\x1b[0m and tabs\tbetween words.";

	group.bench_function("short_fits", |b| {
		b.iter(|| wrap_text_with_ansi_str(std::hint::black_box(short), 80));
	});
	group.bench_function("long_plain_w80", |b| {
		b.iter(|| wrap_text_with_ansi_str(std::hint::black_box(&long_plain), 80));
	});
	group.bench_function("ansi_w80", |b| {
		b.iter(|| wrap_text_with_ansi_str(std::hint::black_box(&ansi_long), 80));
	});
	group.bench_function("cjk_w80", |b| {
		b.iter(|| wrap_text_with_ansi_str(std::hint::black_box(&cjk_text), 80));
	});
	group.bench_function("ansi_tabs_w40", |b| {
		b.iter(|| wrap_text_with_ansi_str(std::hint::black_box(wrapped_tabs), 40));
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// truncate
// ---------------------------------------------------------------------------

fn bench_truncate(c: &mut Criterion) {
	let mut group = c.benchmark_group("truncate");

	let fits = "short";
	let long =
		"This is a rather long line of text that definitely needs to be truncated at some point";
	let ansi_long =
		"\x1b[1;31mError:\x1b[0m something went wrong in \x1b[33mmodule\x1b[0m processing";
	let ansi_sample =
		"\x1b[31mred text\x1b[0m and \x1b[4munderlined content\x1b[24m with emoji \u{1f605}\u{1f605}";

	group.bench_function("fits_early_exit", |b| {
		b.iter(|| {
			truncate_to_width_str(std::hint::black_box(fits), 80, EllipsisKind::Unicode, false)
		});
	});
	group.bench_function("unicode_ellipsis", |b| {
		b.iter(|| {
			truncate_to_width_str(std::hint::black_box(long), 40, EllipsisKind::Unicode, false)
		});
	});
	group.bench_function("ascii_ellipsis", |b| {
		b.iter(|| truncate_to_width_str(std::hint::black_box(long), 40, EllipsisKind::Ascii, false));
	});
	group.bench_function("ansi_truncate", |b| {
		b.iter(|| {
			truncate_to_width_str(std::hint::black_box(ansi_long), 30, EllipsisKind::Unicode, false)
		});
	});
	group.bench_function("ansi_emoji_w32", |b| {
		b.iter(|| {
			truncate_to_width_str(std::hint::black_box(ansi_sample), 32, EllipsisKind::Unicode, false)
		});
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// slice
// ---------------------------------------------------------------------------

fn bench_slice(c: &mut Criterion) {
	let mut group = c.benchmark_group("slice");

	let ascii = "The quick brown fox jumps over the lazy dog near the river bank";
	let ansi =
		"\x1b[31mred text\x1b[0m and \x1b[4munderlined content\x1b[24m with emoji \u{1f605}\u{1f605}";

	group.bench_function("ascii_middle", |b| {
		b.iter(|| slice_with_width_str(std::hint::black_box(ascii), 10, 30, false));
	});
	group.bench_function("ansi_preserve_styles", |b| {
		b.iter(|| slice_with_width_str(std::hint::black_box(ansi), 3, 18, false));
	});
	group.bench_function("extract_segments", |b| {
		b.iter(|| extract_segments_str(std::hint::black_box(ansi), 10, 20, 15, false));
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// sanitize
// ---------------------------------------------------------------------------

fn bench_sanitize(c: &mut Criterion) {
	let mut group = c.benchmark_group("sanitize");

	// Matches oh-mi-pi sanitize.ts test samples
	let plain = "hello world this is a plain ASCII string with some words";
	let ansi =
		"\x1b[31mred text\x1b[0m and \x1b[4munderlined content\x1b[24m with emoji \u{1f605}\u{1f605}";
	let links = "prefix \x1b]8;;https://example.com\x07link\x1b]8;;\x07 suffix";
	let wide = "日本語のテキストとemoji 🚀✨ mixed with ascii";
	let wrapped = "This is a long line that should wrap multiple times when rendered with ANSI \
	               \x1b[32mcolors\x1b[0m and tabs\tbetween words.";
	let control = "hello\x07world\x08back\x01start\x02middle";

	group.bench_function("plain_noop", |b| {
		b.iter(|| sanitize_text_str(std::hint::black_box(plain)));
	});
	group.bench_function("ansi_colors", |b| {
		b.iter(|| sanitize_text_str(std::hint::black_box(ansi)));
	});
	group.bench_function("osc8_links", |b| {
		b.iter(|| sanitize_text_str(std::hint::black_box(links)));
	});
	group.bench_function("cjk_emoji_wide", |b| {
		b.iter(|| sanitize_text_str(std::hint::black_box(wide)));
	});
	group.bench_function("wrapped_tabs", |b| {
		b.iter(|| sanitize_text_str(std::hint::black_box(wrapped)));
	});
	group.bench_function("control_chars", |b| {
		b.iter(|| sanitize_text_str(std::hint::black_box(control)));
	});

	group.finish();
}

criterion_group!(
	benches,
	bench_visible_width,
	bench_wrap,
	bench_truncate,
	bench_slice,
	bench_sanitize
);
criterion_main!(benches);
