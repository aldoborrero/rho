# TUI Architecture Refactor: Eliminate Rc<RefCell<T>>

## Context

The `rho` TUI currently uses `Rc<RefCell<T>>` to share components between `App` (business logic) and `Tui` (rendering). This is needed because `Tui` owns children as `Vec<Box<dyn Component>>` while `interactive.rs` needs mutable access to the same components (e.g., `app.chat.borrow_mut().add_message()`).

The root causes are:
1. `Component::render(&self)` is immutable, but `Editor` needs to mutate scroll state during render
2. `AutocompleteProvider::get_suggestions(&self)` is immutable, but `CombinedAutocompleteProvider` needs to mutate its directory cache
3. `Tui` owns components as trait objects AND the event loop needs mutable access

**Goal:** Replace all `Rc<RefCell<T>>` with direct ownership. App owns components in named struct fields. Tui becomes a pure differential renderer that accepts pre-rendered lines.

## Phase 1: Change `Component::render` from `&self` to `&mut self`

This is the foundational change. Every `Component::render` signature becomes `&mut self`, eliminating the need for `RefCell`-based interior mutability.

### Files to modify

**`crates/rho-tui/src/component.rs`**
- Change `Component::render(&self, width: u16)` to `render(&mut self, width: u16)`
- Change `Container::render(&self, ...)` to `render(&mut self, ...)`
- Update test `TestComponent::render` signature

**`crates/rho-tui/src/tui.rs`**
- `render_children(&self, width)` becomes `render_children(&mut self, width)` (already called from `&mut self` context in `do_render`)
- `composite_overlays(&self, ...)` becomes `composite_overlays(&mut self, ...)` — overlay components need `&mut` for render
- All internal `child.render(width)` calls and `entry.component.render(width)` calls already hold `&mut self`, so no borrow issues

**All `Component` implementors in `rho-tui`** — mechanical signature change (13 total):
- `crates/rho-tui/src/components/editor/mod.rs` — `Editor`
- `crates/rho-tui/src/components/markdown.rs` — `Markdown`
- `crates/rho-tui/src/components/spacer.rs` — `Spacer`
- `crates/rho-tui/src/components/select_list.rs` — `SelectList`
- `crates/rho-tui/src/components/text.rs` — `Text`
- `crates/rho-tui/src/components/truncated_text.rs` — `TruncatedText`
- `crates/rho-tui/src/components/loader.rs` — `Loader` and `CancellableLoader`
- `crates/rho-tui/src/components/input.rs` — `Input`
- `crates/rho-tui/src/components/tab_bar.rs` — `TabBar`
- `crates/rho-tui/src/components/settings_list.rs` — `SettingsList`
- `crates/rho-tui/src/components/padded_box.rs` — `PaddedBox`

**All `Component` implementors in `rho` crate:**
- `crates/rho/src/tui/chat.rs` — `ChatComponent::render(&self)` to `&mut self`
- `crates/rho/src/tui/shared.rs` — `SharedComponent::render(&self)` to `&mut self`, `SharedEditor::render(&self)` to `&mut self`
- `crates/rho/src/tui/welcome.rs` — `WelcomeComponent::render`

### Verification
```bash
cargo build --workspace 2>&1 | head -50
cargo test -p rho-tui
```

## Phase 2: Change `AutocompleteProvider` trait to `&mut self`

**`crates/rho-tui/src/components/editor/mod.rs`**
- Change `AutocompleteProvider::get_suggestions(&self, ...)` to `get_suggestions(&mut self, ...)`
- Change `AutocompleteProvider::get_force_file_suggestions(&self, ...)` to `get_force_file_suggestions(&mut self, ...)`
- Keep `apply_completion`, `get_inline_hint`, `should_trigger_file_completion` as `&self` — they don't need mutation and `get_inline_hint` is called from the render path
- Update call sites: change `let Some(ref provider)` to `let Some(ref mut provider)` for the two mutated methods
- All call sites are in `&mut self` methods (`try_trigger_autocomplete`, `force_file_autocomplete`, `update_autocomplete`) so `&mut` access is available

**`crates/rho/src/tui/autocomplete.rs`**
- `RhoAutocompleteProvider::get_suggestions(&self, ...)` becomes `&mut self`
- `RhoAutocompleteProvider::get_force_file_suggestions(&self, ...)` becomes `&mut self`
- Remove `RefCell<CombinedAutocompleteProvider>` wrapper — change to direct `CombinedAutocompleteProvider` field
- Replace `self.inner.borrow_mut()` with `self.inner` (direct access)
- Replace `self.inner.borrow()` with `&self.inner`

### Verification
```bash
cargo build -p rho-tui -p rho 2>&1 | head -50
cargo test -p rho-tui -p rho
```

## Phase 3: Merge `Editor::render` and `Editor::render_mut`

Now that `Component::render` takes `&mut self`, the two methods can be unified.

**`crates/rho-tui/src/components/editor/mod.rs`**
- Move the scroll-offset and layout-width mutations from `render_mut` into `render(&mut self, ...)`
- Delete `render_mut` method entirely
- The `Component::render` impl now does:
  ```rust
  fn render(&mut self, width: u16) -> Vec<String> {
      let padding_x = self.get_editor_padding_x();
      let layout_width = Self::layout_width(width as usize, padding_x);
      self.last_layout_width = layout_width;
      let layout_lines = layout_text(&self.state, layout_width);
      let visible_height = self.get_visible_content_height(layout_lines.len());
      self.update_scroll_offset(layout_width, &layout_lines, visible_height);
      // ... existing render body ...
  }
  ```

**`crates/rho/src/tui/chat.rs`**
- Replace all `md.render_mut(width)` calls with `md.render(width)` (lines ~142, ~229)

**`crates/rho-tui/benches/markdown.rs`**
- Replace `md.render_mut(...)` with `md.render(...)` in all benchmark functions

### Verification
```bash
cargo build --workspace 2>&1 | head -50
cargo test --workspace
```

## Phase 4: Remove `render_cache: RefCell` from `ChatComponent`

Now that `ChatComponent::render` takes `&mut self`, the render cache no longer needs `RefCell`.

**`crates/rho/src/tui/chat.rs`**
- Change `render_cache: RefCell<HashMap<String, CachedRender>>` to `render_cache: HashMap<String, CachedRender>`
- In `render_tool_result(&mut self, ...)` (was `&self`): replace `self.render_cache.borrow()` with `&self.render_cache` and `self.render_cache.borrow_mut()` with `&mut self.render_cache`
- In `clear()`: replace `self.render_cache.borrow_mut().clear()` with `self.render_cache.clear()`
- In `toggle_tool_expansion()`: same pattern
- In tests: replace `chat.render_cache.borrow()` with `&chat.render_cache`
- Remove `use std::cell::RefCell` import

### Verification
```bash
cargo build -p rho 2>&1 | head -50
cargo test -p rho
```

## Phase 5: Add `InputResult::Submit(String)` variant

Replace the `on_submit` callback with a return value from `handle_input`. This eliminates the mpsc channel indirection for editor submission.

**`crates/rho-tui/src/component.rs`**
- Add variant to `InputResult`:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum InputResult {
      Consumed,
      Ignored,
      Submit(String),
  }
  ```
- Remove `Copy` derive (String is not Copy). Safe: no code copies InputResult values — all usage is immediate construction, early return, or discard. No exhaustive match blocks exist (all components use early-return pattern).

**`crates/rho-tui/src/components/editor/mod.rs`**
- In `submit_value()`: instead of calling `on_submit` callback, return the text
- Change `submit_value()` to return `Option<String>` (None if empty after trim)
- **Two call sites in `handle_input()`** must both be updated:
  - Line ~1288: autocomplete-Enter path (applies autocomplete then immediately submits)
  - Line ~1376: plain Enter handler
- Both should: call `submit_value()`, then return `InputResult::Submit(text)` if Some, or `InputResult::Consumed` if None (editor state reset already happened but text was empty)
- Keep `on_change` callback — it's still fired inside `submit_value()` with empty string to notify text cleared
- Remove `on_submit` field and `TextCallback` type

**`crates/rho-tui/src/tui.rs`**
- In `handle_input()`: propagate `InputResult::Submit` from focused component upward
- Change `handle_input(&mut self, data: &str)` to return `InputResult` (currently returns nothing)

**Update all `InputResult` pattern matches** that assume only 2 variants — search for `InputResult::Consumed` and `InputResult::Ignored` across the workspace. Some matches may need a `Submit` arm or wildcard.

### Verification
```bash
cargo build --workspace 2>&1 | head -50
cargo test --workspace
```

## Phase 6: Rewrite `App` to own components directly

This is the core phase. App owns components as named fields — no more `Rc<RefCell<T>>`.

**`crates/rho/src/tui/mod.rs`**
- Remove `Rc<RefCell<T>>` wrappers from App fields:
  ```rust
  pub struct App {
      pub tui:     Tui,
      pub theme:   Rc<Theme>,  // Keep Rc — theme is shared by closures
      pub chat:    ChatComponent,
      pub status:  StatusLineComponent,
      pub editor:  Editor,
      pub welcome: WelcomeComponent,
  }
  ```
- In `App::new()`:
  - Remove `Rc::new(RefCell::new(...))` wrappers for chat, editor, welcome
  - Stop registering children with `tui.add_child()` — Tui no longer owns components
  - Remove `tui.set_focus()` call (focus is managed by App now)
- Add `App::render_to_tui(&mut self, terminal: &mut dyn Terminal)` method:
  ```rust
  pub fn render_to_tui(&mut self, terminal: &mut dyn Terminal) -> std::io::Result<()> {
      // Render components in order: spacer, welcome, spacer, chat, spacer, editor, spacer
      let width = terminal.columns();
      let mut lines = Vec::new();
      lines.extend(Spacer::new(1).render(width));
      lines.extend(self.welcome.render(width));
      lines.extend(Spacer::new(1).render(width));
      lines.extend(self.chat.render(width));
      lines.extend(Spacer::new(1).render(width));
      lines.extend(self.editor.render(width));
      lines.extend(Spacer::new(1).render(width));
      self.tui.render_lines(&lines, terminal)
  }
  ```
- Add `App::handle_input(&mut self, data: &str) -> InputResult`:
  - Run `tui.process_input_listeners(data)` first — if consumed, return `InputResult::Consumed`
  - Filter key release events (Kitty protocol) unless focused component opts in via `wants_key_release()` — replicate logic from current `Tui::handle_input` lines 260-265
  - Forward to focused component (editor by default)
  - Return the `InputResult` (including `Submit`)
- **Ordering note:** `update_status_border()` must be called *before* `render_to_tui()` since it writes to `self.editor` which `render_to_tui` borrows
- Change `update_status_border` to use `self.editor` directly (no borrow_mut)
- Remove `use shared::{SharedComponent, SharedEditor}` import
- Remove `use std::cell::RefCell` and `use std::rc::Rc` (except Rc for Theme)

### Verification
```bash
cargo build -p rho 2>&1 | head -50
```

## Phase 7: Rewrite `interactive.rs` event loop

Replace all `borrow_mut()` calls with direct field access on `app`.

**`crates/rho/src/modes/interactive.rs`**

- Remove `AppEvent::EditorSubmit` variant entirely
- Remove editor `on_submit` callback setup (the `{ let submit_tx = tx.clone(); ... }` block)
- Remove `use std::cell::RefCell` if present

**Replace all `app.chat.borrow_mut().X()` with `app.chat.X()`:**
- `app.chat.borrow_mut().add_message(...)` → `app.chat.add_message(...)`
- `app.chat.borrow_mut().start_streaming()` → `app.chat.start_streaming()`
- `app.chat.borrow_mut().finish_streaming()` → `app.chat.finish_streaming()`
- `app.chat.borrow_mut().append_text(...)` → `app.chat.append_text(...)`
- `app.chat.borrow_mut().append_thinking(...)` → `app.chat.append_thinking(...)`
- `app.chat.borrow_mut().clear()` → `app.chat.clear()`
- `app.chat.borrow_mut().toggle_tool_expansion()` → `app.chat.toggle_tool_expansion()`

**Replace all `app.editor.borrow_mut().X()` with `app.editor.X()`:**
- `app.editor.borrow_mut().add_to_history(...)` → `app.editor.add_to_history(...)`

**Change input handling to use `InputResult::Submit`:**
```rust
// In Terminal input handling, replace `app.tui.handle_input(data)` with:
let result = app.handle_input(data);
if let InputResult::Submit(text) = result {
    // Handle submission inline (same logic as old EditorSubmit handler)
    app.editor.add_to_history(&text);
    match route_input(&text) { ... }
}
```

**Replace the `Some(AppEvent::EditorSubmit(text))` match arm** — move its body into the Submit handler above.

**`show_chat_message` helper:**
- Change from `app.chat.borrow_mut().add_message(...)` to `app.chat.add_message(...)`

**`apply_command_result` helper:**
- Same pattern — replace all `borrow_mut()` with direct access

### Tui changes needed

**`crates/rho-tui/src/tui.rs`**
- Add `pub fn render_lines(&mut self, content_lines: &[String], terminal: &mut dyn Terminal) -> std::io::Result<()>`:
  ```rust
  pub fn render_lines(&mut self, content_lines: &[String], terminal: &mut dyn Terminal) -> std::io::Result<()> {
      if !self.render_needed || self.stopped { return Ok(()); }
      self.render_needed = false;
      let width = terminal.columns();
      let height = terminal.rows();
      let mut new_lines = content_lines.to_vec();
      // Overlay compositing still happens here (Tui owns the overlay stack)
      if !self.overlay_stack.is_empty() {
          self.composite_overlays(&mut new_lines, width, height);
      }
      // ... cursor extraction, line resets, differential rendering (reuse do_render internals) ...
  }
  ```
- Add `pub fn process_input_listeners(&mut self, data: &str) -> Option<String>` — runs input listeners, returns Some(transformed data) to forward, or None if consumed
- Remove `children`, `focused_idx` fields
- Remove `add_child()`, `clear_children()`, `set_focus()`, `focused_component()`, `focused_component_mut()`, `render_children()` methods
- Update `invalidate()` to only iterate over `overlay_stack` (App calls `invalidate()` on its own components separately)

### Verification
```bash
cargo build -p rho 2>&1 | head -50
cargo test -p rho
```

## Phase 8: Delete `shared.rs` and clean up

**`crates/rho/src/tui/shared.rs`** — Delete entirely

**`crates/rho/src/tui/mod.rs`**
- Remove `pub mod shared;` declaration
- Remove any remaining `SharedComponent` / `SharedEditor` imports

**`crates/rho-tui/src/tui.rs`**
- Verify `children` and `focused_idx` fields are already removed (done in Phase 7)
- Verify all deprecated methods are removed
- `Tui::invalidate()` now only iterates `overlay_stack` entries (children loop already gone)
- Clean up `Tui::new()` to not initialize removed fields

**`crates/rho-tui/src/components/editor/mod.rs`**
- Remove `on_submit` field if not used elsewhere
- Remove `TextCallback` type alias if unused
- Remove `render_mut` method if not already removed in Phase 3

### Verification
```bash
cargo clippy --workspace 2>&1 | head -80
cargo test --workspace
cargo bench --no-run  # Ensure benchmarks still compile
```

## File summary

| File | Action |
|------|--------|
| `crates/rho-tui/src/component.rs` | `render(&self)` → `render(&mut self)`, add `Submit(String)` to `InputResult` |
| `crates/rho-tui/src/tui.rs` | Add `render_lines()`, `process_input_listeners()`. Remove children/focus fields. Update `invalidate()` |
| `crates/rho-tui/src/components/editor/mod.rs` | Merge render_mut into render, return `Submit` from handle_input, change AutocompleteProvider to `&mut self` |
| `crates/rho-tui/src/components/spacer.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/src/components/markdown.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/src/components/select_list.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/src/components/text.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/src/components/truncated_text.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/src/components/loader.rs` | `render(&self)` → `render(&mut self)` (Loader + CancellableLoader) |
| `crates/rho-tui/src/components/input.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/src/components/tab_bar.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/src/components/settings_list.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/src/components/padded_box.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho-tui/benches/markdown.rs` | `render_mut` → `render` |
| `crates/rho/src/tui/mod.rs` | Rewrite App: direct ownership, remove Rc<RefCell>, add render_to_tui/handle_input |
| `crates/rho/src/tui/shared.rs` | **Delete** |
| `crates/rho/src/tui/chat.rs` | Remove RefCell from render_cache, update render signature |
| `crates/rho/src/tui/autocomplete.rs` | Remove RefCell from inner, change to &mut self |
| `crates/rho/src/tui/welcome.rs` | `render(&self)` → `render(&mut self)` |
| `crates/rho/src/modes/interactive.rs` | Remove EditorSubmit event, remove all borrow_mut(), inline submit handling |

## End-to-end verification

```bash
# Full build
cargo build --workspace

# All tests
cargo test --workspace

# Clippy (strict workspace lints)
cargo clippy --workspace

# Benchmarks compile
cargo bench --no-run

# Manual smoke test
cargo run -p rho
# Type a message, verify rendering, Ctrl+C to exit
```
