use criterion::{Criterion, criterion_group, criterion_main};
use rho_tui::StdinBuffer;

// ---------------------------------------------------------------------------
// StdinBuffer::process
// ---------------------------------------------------------------------------

fn bench_process(c: &mut Criterion) {
	let mut group = c.benchmark_group("process");

	// Single character
	group.bench_function("single_char", |b| {
		let mut buf = StdinBuffer::new();
		b.iter(|| {
			buf.process(std::hint::black_box(b"a"));
		});
	});

	// Multi-byte burst
	group.bench_function("multi_byte_burst", |b| {
		let mut buf = StdinBuffer::new();
		b.iter(|| {
			buf.process(std::hint::black_box(b"hello world"));
		});
	});

	// Complete escape sequence (arrow up)
	group.bench_function("escape_sequence", |b| {
		let mut buf = StdinBuffer::new();
		b.iter(|| {
			buf.process(std::hint::black_box(b"\x1b[A"));
		});
	});

	// Mouse event SGR
	group.bench_function("mouse_sgr", |b| {
		let mut buf = StdinBuffer::new();
		b.iter(|| {
			buf.process(std::hint::black_box(b"\x1b[<0;10;5M"));
		});
	});

	// Kitty keyboard event
	group.bench_function("kitty_keyboard", |b| {
		let mut buf = StdinBuffer::new();
		b.iter(|| {
			buf.process(std::hint::black_box(b"\x1b[97;1:1u"));
		});
	});

	// Bracketed paste, short
	group.bench_function("paste_short", |b| {
		let mut buf = StdinBuffer::new();
		let input = b"\x1b[200~short paste\x1b[201~";
		b.iter(|| {
			buf.process(std::hint::black_box(input));
		});
	});

	// Bracketed paste, long (1KB content)
	group.bench_function("paste_1kb", |b| {
		let mut buf = StdinBuffer::new();
		let content = "x".repeat(1024);
		let input = format!("\x1b[200~{content}\x1b[201~");
		let input_bytes = input.as_bytes();
		b.iter(|| {
			buf.process(std::hint::black_box(input_bytes));
		});
	});

	// Mixed: chars + escape sequences interleaved
	group.bench_function("mixed_interleaved", |b| {
		let mut buf = StdinBuffer::new();
		let input = b"abc\x1b[Adef\x1b[B\x1b[97ughi";
		b.iter(|| {
			buf.process(std::hint::black_box(input));
		});
	});

	group.finish();
}

criterion_group!(benches, bench_process);
criterion_main!(benches);
