use criterion::{Criterion, criterion_group, criterion_main};
use rho_tui::keys::{matches_key, parse_key, parse_kitty_sequence};

// ---------------------------------------------------------------------------
// parse_key — covers the full oh-mi-pi parse-key.ts sample set
// ---------------------------------------------------------------------------

fn bench_parse_key(c: &mut Criterion) {
	let mut group = c.benchmark_group("parse_key");

	// --- Printable characters (fast path) ---
	group.bench_function("ascii_a", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"a"), false));
	});
	group.bench_function("ascii_z", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"z"), false));
	});
	group.bench_function("ascii_slash", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"/"), false));
	});

	// --- Single-byte special keys ---
	group.bench_function("escape", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b"), false));
	});
	group.bench_function("tab", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\t"), false));
	});
	group.bench_function("enter", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\r"), false));
	});
	group.bench_function("space", |b| {
		b.iter(|| parse_key(std::hint::black_box(b" "), false));
	});
	group.bench_function("backspace", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x7f"), false));
	});

	// --- Ctrl+key ---
	group.bench_function("ctrl_c", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x03"), false));
	});
	group.bench_function("ctrl_z", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1a"), false));
	});
	group.bench_function("ctrl_space", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x00"), false));
	});

	// --- Legacy escape sequences (PHF lookup) ---
	group.bench_function("legacy_shift_tab", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[Z"), false));
	});
	group.bench_function("legacy_up", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[A"), false));
	});
	group.bench_function("legacy_down", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[B"), false));
	});
	group.bench_function("legacy_left", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[D"), false));
	});
	group.bench_function("legacy_right", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[C"), false));
	});
	group.bench_function("legacy_home", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[H"), false));
	});
	group.bench_function("legacy_end", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[F"), false));
	});
	group.bench_function("legacy_delete", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[3~"), false));
	});
	group.bench_function("legacy_page_up", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[5~"), false));
	});
	group.bench_function("legacy_page_down", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[6~"), false));
	});

	// --- Legacy function keys ---
	group.bench_function("legacy_f1", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1bOP"), false));
	});
	group.bench_function("legacy_f5", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[15~"), false));
	});
	group.bench_function("legacy_f12", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[24~"), false));
	});

	// --- Alt+key escape pairs ---
	group.bench_function("alt_backspace", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b\x7f"), false));
	});
	group.bench_function("alt_left", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1bb"), false));
	});
	group.bench_function("alt_right", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1bf"), false));
	});

	// --- Legacy modified keys ---
	group.bench_function("legacy_shift_up", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[a"), false));
	});
	group.bench_function("legacy_ctrl_up", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1bOa"), false));
	});

	// --- Kitty protocol sequences ---
	group.bench_function("kitty_simple_a", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[97u"), true));
	});
	group.bench_function("kitty_ctrl_a", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[97;5u"), true));
	});
	group.bench_function("kitty_shift_tab", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[9;2u"), true));
	});
	group.bench_function("kitty_alt_enter", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[13;3u"), true));
	});
	group.bench_function("kitty_ctrl_right", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[1;5C"), true));
	});
	group.bench_function("kitty_shift_delete", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[3;2~"), true));
	});
	group.bench_function("kitty_base_layout", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[108::97;5u"), true));
	});
	group.bench_function("kitty_release_event", |b| {
		b.iter(|| parse_key(std::hint::black_box(b"\x1b[97;1:3u"), true));
	});

	// --- Batch: all 36 samples in a single iteration (throughput test) ---
	group.bench_function("batch_all_legacy", |b| {
		let samples: &[&[u8]] = &[
			b"\x1b",
			b"\t",
			b"\r",
			b" ",
			b"\x7f",
			b"\x1b[Z",
			b"\x1b[A",
			b"\x1b[B",
			b"\x1b[D",
			b"\x1b[C",
			b"\x1b[H",
			b"\x1b[F",
			b"\x1b[3~",
			b"\x1b[5~",
			b"\x1b[6~",
			b"\x1bOP",
			b"\x1b[15~",
			b"\x1b[24~",
			b"\x03",
			b"\x1a",
			b"\x00",
			b"\x1b\x7f",
			b"\x1bb",
			b"\x1bf",
			b"\x1b[a",
			b"\x1bOa",
			b"a",
			b"z",
			b"/",
		];
		b.iter(|| {
			for &s in samples {
				std::hint::black_box(parse_key(std::hint::black_box(s), false));
			}
		});
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// matches_key — covers the oh-mi-pi parse-key.ts match test set
// ---------------------------------------------------------------------------

fn bench_matches_key(c: &mut Criterion) {
	let mut group = c.benchmark_group("matches_key");

	// --- Printable character matches ---
	group.bench_function("ascii_a", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"a"), std::hint::black_box("a"), false));
	});
	group.bench_function("ascii_slash", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"/"), std::hint::black_box("/"), false));
	});

	// --- Single-byte special keys ---
	group.bench_function("escape", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x1b"), std::hint::black_box("escape"), false));
	});
	group.bench_function("tab", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\t"), std::hint::black_box("tab"), false));
	});
	group.bench_function("enter", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\r"), std::hint::black_box("enter"), false));
	});
	group.bench_function("backspace", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x7f"), std::hint::black_box("backspace"), false)
		});
	});

	// --- Ctrl+key ---
	group.bench_function("ctrl_c", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x03"), std::hint::black_box("ctrl+c"), false));
	});
	group.bench_function("ctrl_z", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x1a"), std::hint::black_box("ctrl+z"), false));
	});
	group.bench_function("ctrl_space", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x00"), std::hint::black_box("ctrl+space"), false)
		});
	});

	// --- Legacy sequence matches ---
	group.bench_function("legacy_up", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x1b[A"), std::hint::black_box("up"), false));
	});
	group.bench_function("legacy_down", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x1b[B"), std::hint::black_box("down"), false));
	});
	group.bench_function("legacy_right", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x1b[C"), std::hint::black_box("right"), false));
	});
	group.bench_function("legacy_delete", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1b[3~"), std::hint::black_box("delete"), false)
		});
	});
	group.bench_function("legacy_f1", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x1bOP"), std::hint::black_box("f1"), false));
	});
	group.bench_function("legacy_f12", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x1b[24~"), std::hint::black_box("f12"), false));
	});

	// --- Alt+key matches ---
	group.bench_function("alt_backspace", |b| {
		b.iter(|| {
			matches_key(
				std::hint::black_box(b"\x1b\x7f"),
				std::hint::black_box("alt+backspace"),
				false,
			)
		});
	});
	group.bench_function("alt_left", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1bb"), std::hint::black_box("alt+left"), false)
		});
	});
	group.bench_function("alt_right", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1bf"), std::hint::black_box("alt+right"), false)
		});
	});

	// --- Kitty protocol matches ---
	group.bench_function("kitty_ctrl_a", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1b[97;5u"), std::hint::black_box("ctrl+a"), true)
		});
	});
	group.bench_function("kitty_shift_tab", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1b[9;2u"), std::hint::black_box("shift+tab"), true)
		});
	});
	group.bench_function("kitty_alt_enter", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1b[13;3u"), std::hint::black_box("alt+enter"), true)
		});
	});
	group.bench_function("kitty_ctrl_right", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1b[1;5C"), std::hint::black_box("ctrl+right"), true)
		});
	});
	group.bench_function("kitty_shift_delete", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1b[3;2~"), std::hint::black_box("shift+delete"), true)
		});
	});

	// --- text-layout.ts matchesKey samples ---
	group.bench_function("match_arrow_up", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"\x1b[A"), std::hint::black_box("up"), false));
	});
	group.bench_function("match_ctrl_right", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1b[1;5C"), std::hint::black_box("ctrl+right"), false)
		});
	});
	group.bench_function("match_shift_left", |b| {
		b.iter(|| {
			matches_key(std::hint::black_box(b"\x1b[1;2D"), std::hint::black_box("shift+left"), false)
		});
	});

	// --- Miss path ---
	group.bench_function("miss", |b| {
		b.iter(|| matches_key(std::hint::black_box(b"a"), std::hint::black_box("z"), false));
	});

	// --- Batch: all matches in a single iteration ---
	group.bench_function("batch_all", |b| {
		let samples: &[(&[u8], &str, bool)] = &[
			(b"\x1b", "escape", false),
			(b"\t", "tab", false),
			(b"\r", "enter", false),
			(b" ", "space", false),
			(b"\x7f", "backspace", false),
			(b"\x1b[A", "up", false),
			(b"\x1b[B", "down", false),
			(b"\x1b[C", "right", false),
			(b"\x1b[D", "left", false),
			(b"\x1b[3~", "delete", false),
			(b"\x03", "ctrl+c", false),
			(b"\x1a", "ctrl+z", false),
			(b"\x1b\x7f", "alt+backspace", false),
			(b"\x1b[97;5u", "ctrl+a", true),
			(b"\x1b[9;2u", "shift+tab", true),
			(b"\x1b[13;3u", "alt+enter", true),
			(b"\x1b[1;5C", "ctrl+right", true),
			(b"\x1b[3;2~", "shift+delete", true),
			(b"a", "a", false),
			(b"z", "z", false),
		];
		b.iter(|| {
			for &(bytes, key, kitty) in samples {
				std::hint::black_box(matches_key(
					std::hint::black_box(bytes),
					std::hint::black_box(key),
					kitty,
				));
			}
		});
	});

	group.finish();
}

// ---------------------------------------------------------------------------
// kitty_sequence — dedicated Kitty protocol parsing (kitty-sequence.ts parity)
// ---------------------------------------------------------------------------

fn bench_kitty_sequence(c: &mut Criterion) {
	let mut group = c.benchmark_group("kitty_sequence");

	// ctrl+a
	group.bench_function("ctrl_a", |b| {
		b.iter(|| parse_kitty_sequence(std::hint::black_box(b"\x1b[97;5u")));
	});
	// shift+tab
	group.bench_function("shift_tab", |b| {
		b.iter(|| parse_kitty_sequence(std::hint::black_box(b"\x1b[9;2u")));
	});
	// alt+enter
	group.bench_function("alt_enter", |b| {
		b.iter(|| parse_kitty_sequence(std::hint::black_box(b"\x1b[13;3u")));
	});
	// ctrl+right (CSI-style with letter terminator)
	group.bench_function("ctrl_right", |b| {
		b.iter(|| parse_kitty_sequence(std::hint::black_box(b"\x1b[1;5C")));
	});
	// shift+delete (tilde-terminated)
	group.bench_function("shift_delete", |b| {
		b.iter(|| parse_kitty_sequence(std::hint::black_box(b"\x1b[3;2~")));
	});
	// base-layout mapping
	group.bench_function("base_layout", |b| {
		b.iter(|| parse_kitty_sequence(std::hint::black_box(b"\x1b[108::97;5u")));
	});

	// Batch: all 6 in a single iteration
	group.bench_function("batch_all", |b| {
		let samples: &[&[u8]] = &[
			b"\x1b[97;5u",
			b"\x1b[9;2u",
			b"\x1b[13;3u",
			b"\x1b[1;5C",
			b"\x1b[3;2~",
			b"\x1b[108::97;5u",
		];
		b.iter(|| {
			for &s in samples {
				std::hint::black_box(parse_kitty_sequence(std::hint::black_box(s)));
			}
		});
	});

	group.finish();
}

criterion_group!(benches, bench_parse_key, bench_matches_key, bench_kitty_sequence);
criterion_main!(benches);
