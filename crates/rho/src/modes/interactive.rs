use rho_agent::{
	agent_loop::{AgentConfig, ThinkingLevel},
	events::{AgentEvent, AgentOutcome},
};
use rho_tui::{Terminal, component::InputResult};

use crate::{
	ai::types::{AssistantMessage, ContentBlock, Message, ToolResultMessage, UserMessage},
	cli::Cli,
	commands::{CommandContext, CommandResult},
	models_config::{ModelsConfig, ResolvedModel},
	modes::{
		input::{InputAction, route_input},
		state::AppMode,
	},
	session::SessionManager,
	settings::Settings,
	tools::registry::ToolRegistry,
	tui,
};

/// An [`AgentEvent`] tagged with the generation that produced it.
///
/// The generation counter prevents stale events from an old (cancelled) agent
/// run from corrupting the session when a new agent run has already started.
struct TaggedAgentEvent {
	generation: u64,
	event:      AgentEvent,
}

/// Events dispatched through the main event loop.
enum AppEvent {
	/// Raw terminal input (key press, paste, resize).
	Terminal(rho_tui::TerminalEvent),
	/// Agent event from the autonomous agent loop.
	Agent(TaggedAgentEvent),
	/// Periodic tick for spinner animation.
	Tick,
	/// A chunk of output from a bang command.
	BangChunk(String),
	/// Bang command completed.
	BangDone { is_error: bool },
}

/// Map a tool name to a human-readable phase label for the status line.
fn generate_tool_phase(tool_name: &str) -> String {
	match tool_name {
		"read" => "Reading file".to_owned(),
		"write" => "Writing file".to_owned(),
		"edit" => "Editing file".to_owned(),
		"bash" => "Running bash".to_owned(),
		"grep" => "Searching files".to_owned(),
		"find" | "glob" => "Finding files".to_owned(),
		"fuzzy_find" => "Fuzzy finding".to_owned(),
		"clipboard" => "Using clipboard".to_owned(),
		_ => format!("Running {tool_name}"),
	}
}

/// Cancel the active streaming agent and reset UI state.
///
/// Increments the generation counter so that any remaining events from the
/// old agent run are silently dropped.
fn cancel_streaming(
	mode: &mut AppMode,
	app: &mut tui::App,
	terminal: &impl Terminal,
	agent_generation: &mut u64,
	agent_cancel: &mut Option<tokio_util::sync::CancellationToken>,
) -> Option<crate::ai::types::AssistantMessage> {
	*mode = AppMode::Idle;
	*agent_generation += 1;
	if let Some(token) = agent_cancel.take() {
		token.cancel();
	}
	let partial = app.chat.commit_partial_streaming();
	app.chat.finish_streaming();
	app.status.finish_working();
	app.update_status_border(terminal.columns());
	partial
}

/// Cancel the running bang command and reset UI state.
fn cancel_bang(
	mode: &mut AppMode,
	app: &mut tui::App,
	bang_cancel: &mut Option<tokio::sync::oneshot::Sender<()>>,
) {
	if let Some(cancel) = bang_cancel.take() {
		let _ = cancel.send(());
	}
	app.chat.finish_bang(true);
	*mode = AppMode::Idle;
}

/// Parse a thinking level string into a `ThinkingLevel`.
fn parse_thinking(s: &str) -> ThinkingLevel {
	match s {
		"low" => ThinkingLevel::Low,
		"medium" => ThinkingLevel::Medium,
		"high" => ThinkingLevel::High,
		_ => ThinkingLevel::Off,
	}
}

/// Spawn the autonomous agent loop in a background task.
///
/// Agent events are forwarded as `AppEvent::Agent` through the given channel,
/// tagged with the current generation so stale events can be discarded.
///
/// The `agent_generation` counter is incremented before spawning so that any
/// events from a previous agent run are automatically ignored by the event
/// loop's generation check.
fn spawn_agent(
	model: &rho_ai::Model,
	messages: &[Message],
	tools: &ToolRegistry,
	system_prompt: &str,
	settings: &Settings,
	api_key: &str,
	tx: &tokio::sync::mpsc::Sender<AppEvent>,
	agent_generation: &mut u64,
) -> tokio_util::sync::CancellationToken {
	*agent_generation += 1;
	let generation = *agent_generation;
	let cancel = tokio_util::sync::CancellationToken::new();

	let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);

	// Forward agent events as AppEvent::Agent, tagged with the generation.
	let forward_tx = tx.clone();
	tokio::spawn(async move {
		while let Some(event) = agent_rx.recv().await {
			if forward_tx
				.send(AppEvent::Agent(TaggedAgentEvent { generation, event }))
				.await
				.is_err()
			{
				break;
			}
		}
	});

	// Run agent loop
	let agent_model = model.clone();
	let agent_tools = tools.clone();
	let agent_config = AgentConfig {
		system_prompt: system_prompt.to_owned(),
		max_tokens:    settings.agent.max_tokens,
		thinking:      parse_thinking(&settings.agent.thinking),
		retry:         rho_ai::RetryConfig {
			enabled:       true,
			max_retries:   settings.retry.max_retries,
			base_delay_ms: settings.retry.base_delay_ms,
			max_delay_ms:  settings.retry.base_delay_ms * 16,
		},
		cwd:           std::env::current_dir().unwrap_or_default(),
		api_key:       Some(api_key.to_owned()),
		temperature:   Some(settings.agent.temperature),
		abort:         Some(cancel.clone()),
	};
	let mut agent_messages = messages.to_vec();
	tokio::spawn(async move {
		let _outcome = rho_agent::agent_loop::run_agent_loop(
			&agent_model,
			&mut agent_messages,
			&agent_tools,
			agent_config,
			agent_tx,
		)
		.await;
	});

	cancel
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Add a system/informational message to the chat display.
fn show_chat_message(app: &mut tui::App, text: &str) {
	app.chat.add_message(Message::Assistant(AssistantMessage {
		content:     vec![ContentBlock::Text { text: text.to_owned() }],
		stop_reason: None,
		usage:       None,
	}));
}

/// Outcome of applying a [`CommandResult`].
enum ApplyOutcome {
	/// The command was handled; continue the event loop.
	Handled,
	/// The command requested an exit.
	Exit,
}

/// Apply a [`CommandResult`] to the application state.
///
/// Returns [`ApplyOutcome::Exit`] if the event loop should break.
#[allow(
	clippy::future_not_send,
	clippy::too_many_arguments,
	reason = "App contains non-Send TUI components; runs on the main task only"
)]
async fn apply_command_result(
	result: CommandResult,
	session: &mut SessionManager,
	app: &mut tui::App,
	terminal: &impl Terminal,
	model: &mut rho_ai::Model,
	api_key: &mut String,
	settings: &mut Settings,
	registry: &rho_ai::ModelRegistry,
	models_config: &ModelsConfig,
	cli: &Cli,
) -> anyhow::Result<ApplyOutcome> {
	match result {
		CommandResult::Message(msg) => {
			show_chat_message(app, &msg);
		},
		CommandResult::Exit => return Ok(ApplyOutcome::Exit),
		CommandResult::NewSession => {
			session.clear().await?;
			app.chat.clear();
		},
		CommandResult::ChangeDir(path) => {
			std::env::set_current_dir(&path)?;
			show_chat_message(app, &format!("Working directory changed to: {path}"));
		},
		CommandResult::Fork => match session.fork() {
			Ok(_) => {
				app.status.set_session_id(session.session_id());
				app.update_status_border(terminal.columns());
				show_chat_message(app, &format!("Session forked: {}", session.session_id()));
			},
			Err(e) => {
				show_chat_message(app, &format!("Fork failed: {e}"));
			},
		},
		CommandResult::Compact(instructions) => {
			show_chat_message(app, "Compacting conversation...");
			let compaction_settings = &settings.compaction;
			match crate::compaction::compact::run_compaction(
				session,
				model,
				&settings.api_key,
				compaction_settings,
				instructions.as_deref(),
			)
			.await
			{
				Ok(result) => {
					session.append_compaction(
						&result.summary,
						result.short_summary.as_deref(),
						&result.first_kept_entry_id,
						result.tokens_before,
						result.details,
					)?;
					let msg = result
						.short_summary
						.as_deref()
						.unwrap_or("Conversation compacted.");
					show_chat_message(app, &format!("Compacted: {msg}"));
				},
				Err(e) => show_chat_message(app, &format!("Compaction failed: {e}")),
			}
		},
		CommandResult::ModelChange(new_model) => {
			match crate::models_config::resolve_model(
				&new_model, registry, settings, models_config,
			) {
				Ok(resolved) => {
					let old_id = model.id.clone();
					*model = resolved.model;
					*api_key = resolved.api_key;
					app.status.set_model(&model.id);
					app.update_status_border(terminal.columns());
					show_chat_message(
						app,
						&format!("Model changed: {old_id} → {}", model.id),
					);
				},
				Err(e) => {
					show_chat_message(app, &format!("Failed to switch model: {e}"));
				},
			}
		},
		CommandResult::SettingsChanged => {
			match crate::settings::reload(cli) {
				Ok(new_settings) => {
					*settings = new_settings;
					show_chat_message(app, "Settings reloaded.");
				},
				Err(e) => {
					show_chat_message(app, &format!("Failed to reload settings: {e}"));
				},
			}
		},
		CommandResult::Silent => {},
	}
	Ok(ApplyOutcome::Handled)
}

/// Run the interactive TUI mode.
///
/// This sets up the terminal, spawns a background thread for reading terminal
/// events, and runs the main event loop that dispatches between terminal input,
/// agent events, and editor submit handling.
///
/// The app shows the editor, accepts input, and exits on Ctrl+C or Ctrl+D.
#[allow(
	clippy::future_not_send,
	reason = "App contains non-Send TUI components; runs on the main task only"
)]
pub async fn run_interactive(
	cli: &Cli,
	mut settings: Settings,
	models_config: ModelsConfig,
	registry: rho_ai::ModelRegistry,
	resolved: ResolvedModel,
	mut session: SessionManager,
	tools: ToolRegistry,
) -> anyhow::Result<()> {
	// Load session messages if resuming (now a no-op since open() loads).
	session.load().await?;

	// Write breadcrumb linking this terminal to the session file.
	if let Some(file) = session.session_file() {
		let cwd = std::env::current_dir().unwrap_or_default();
		crate::session::breadcrumb::write_breadcrumb(&cwd, file);
	}

	// Unpack the resolved model.
	let mut model = resolved.model;
	let mut api_key = resolved.api_key;

	// Build the system prompt once.
	let system_prompt = crate::prompts::build(&tools, crate::prompts::BuildOptions {
		custom_prompt:        cli.system_prompt.clone(),
		append_system_prompt: cli.append_system_prompt.clone(),
		cwd:                  std::env::current_dir().unwrap_or_default(),
	})
	.await?;

	// Start terminal in raw mode.
	let mut terminal = tui::start_terminal()?;

	// Set up panic handler to restore terminal on panic.
	let original_hook = std::panic::take_hook();
	std::panic::set_hook(Box::new(move |info| {
		rho_tui::emergency_terminal_restore();
		original_hook(info);
	}));

	// Create the TUI application.
	let mut app = tui::App::new(&model.id);

	// Set session id on the status line.
	app.status.set_session_id(session.session_id());
	app.update_status_border(80); // Initial status border

	// Create the event channel.
	let (tx, mut rx) = tokio::sync::mpsc::channel::<AppEvent>(64);

	// Spawn terminal event reader on a blocking thread.
	//
	// We use crossterm's event::poll/read directly (thread-safe) rather than
	// going through CrosstermTerminal::poll_event, because the terminal instance
	// is needed on the main task for rendering.
	let term_tx = tx.clone();
	let _term_handle = tokio::task::spawn_blocking(move || {
		loop {
			// Poll crossterm events with 50ms timeout.
			if crossterm::event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
				match crossterm::event::read() {
					Ok(event) => {
						let app_event = match event {
							crossterm::event::Event::Key(key) => {
								let input = crossterm_key_to_string(&key);
								if input.is_empty() {
									continue;
								}
								AppEvent::Terminal(rho_tui::TerminalEvent::Input(input))
							},
							crossterm::event::Event::Paste(text) => {
								AppEvent::Terminal(rho_tui::TerminalEvent::Paste(text))
							},
							crossterm::event::Event::Resize(cols, rows) => {
								AppEvent::Terminal(rho_tui::TerminalEvent::Resize(cols, rows))
							},
							_ => continue,
						};
						if term_tx.blocking_send(app_event).is_err() {
							break; // Channel closed, exit.
						}
					},
					Err(_) => break,
				}
			}
		}
	});

	// Render existing session messages (if resuming).
	for msg in session.messages() {
		app.chat.add_message(msg.clone());
	}

	// Track application mode.
	let mut mode = AppMode::Idle;
	// Generation counter: incremented on every agent spawn and cancel.
	// Events from old generations are silently dropped.
	let mut agent_generation: u64 = 0;
	// Cancellation token for the currently running agent loop.
	let mut agent_cancel: Option<tokio_util::sync::CancellationToken> = None;
	// Cancellation handle for the currently running bang command.
	let mut bang_cancel: Option<tokio::sync::oneshot::Sender<()>> = None;

	// If an initial message was provided on the command line, send it immediately.
	if let Some(initial_text) = cli.initial_message() {
		let user_msg = Message::User(UserMessage { content: initial_text });
		app.chat.add_message(user_msg.clone());
		session.append(user_msg).await?;

		mode = AppMode::Streaming;
		app.chat.start_streaming();
		app.status.clear_work_status();
		app.status.start_working();
		app.update_status_border(terminal.columns());
		agent_cancel = Some(spawn_agent(&model, session.messages(), &tools, &system_prompt, &settings, &api_key, &tx, &mut agent_generation));
	}

	// Perform the initial render.
	app.tui.request_render();
	app.render_to_tui(&mut terminal)?;

	// Tick interval for spinner animation.
	let mut tick_interval = tokio::time::interval(std::time::Duration::from_millis(80));
	tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

	// Main event loop.
	loop {
		let event = tokio::select! {
			event = rx.recv() => event,
			_ = tick_interval.tick() => Some(AppEvent::Tick),
		};
		match event {
			Some(AppEvent::Tick) => {
				let needs_render = app.chat.tick();
				if app.status.is_working() {
					app.update_status_border(terminal.columns());
					app.tui.request_render();
					app.render_to_tui(&mut terminal)?;
				} else if needs_render {
					app.tui.request_render();
					app.render_to_tui(&mut terminal)?;
				}
				continue;
			},
			Some(AppEvent::Terminal(event)) => match event {
				rho_tui::TerminalEvent::Input(ref data) => {
					// Ctrl+C
					if data == "\x03" {
						if matches!(mode, AppMode::Streaming) {
							if let Some(partial) = cancel_streaming(&mut mode, &mut app, &terminal, &mut agent_generation, &mut agent_cancel) {
								session.append(Message::Assistant(partial)).await?;
							}
						} else if matches!(mode, AppMode::BangRunning) {
							cancel_bang(&mut mode, &mut app, &mut bang_cancel);
						} else {
							break;
						}
					}
					// Ctrl+D
					else if data == "\x04" {
						break;
					}
					// Ctrl+L: clear chat display.
					else if data == "\x0c" {
						app.chat.clear();
					}
					// Ctrl+O: toggle tool output expansion.
					else if data == "\x0f" {
						app.chat.toggle_tool_expansion();
					}
					// Escape — cancel active operation (bypasses editor).
					else if data == "\x1b"
						&& matches!(mode, AppMode::Streaming | AppMode::BangRunning)
					{
						if matches!(mode, AppMode::Streaming) {
							if let Some(partial) = cancel_streaming(&mut mode, &mut app, &terminal, &mut agent_generation, &mut agent_cancel) {
								session.append(Message::Assistant(partial)).await?;
							}
						} else {
							cancel_bang(&mut mode, &mut app, &mut bang_cancel);
						}
					}
					// Forward other input to the app (routes through input
					// listeners then to the editor).
					else {
						let result = app.handle_input(data);
						app.sync_editor_border_color();
						if let InputResult::Submit(text) = result {
							// Block submissions while a bang command is running.
							if matches!(mode, AppMode::BangRunning) {
								continue;
							}

							// Handle submission inline.
							app.editor.add_to_history(&text);

							match route_input(&text) {
								InputAction::Empty => {},
								InputAction::SlashCommand { name, args } => {
									let ctx = CommandContext {
										name,
										args,
										session: &session,
										settings: &settings,
										model: &model.id,
										tools: &tools,
									};
									let result = crate::commands::execute_command(&ctx).await?;
									if matches!(
										apply_command_result(
											result,
											&mut session,
											&mut app,
											&terminal,
											&mut model,
											&mut api_key,
											&mut settings,
											&registry,
											&models_config,
											cli,
										)
										.await?,
										ApplyOutcome::Exit
									) {
										break;
									}
								},
								InputAction::UnknownCommand(cmd) => {
									show_chat_message(
										&mut app,
										&format!(
											"Unknown command: {cmd}. Type /help for available commands."
										),
									);
								},
								InputAction::BangCommand(cmd) => {
									// Show the user's command in chat.
									app.chat.add_message(Message::User(UserMessage {
										content: format!("!{cmd}"),
									}));

									// Start a streaming bang output block.
									app.chat.start_bang(cmd);
									mode = AppMode::BangRunning;

									// Spawn background execution with cancellation support.
									let chunk_tx = tx.clone();
									let done_tx = tx.clone();
									let bang_tools = tools.clone();
									let cmd_owned = cmd.to_owned();
									let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
									bang_cancel = Some(cancel_tx);
									tokio::spawn(async move {
										let shell_fut = crate::commands::execute_bang_streaming(
											&cmd_owned,
											&bang_tools,
											move |chunk| {
												// try_send avoids blocking a tokio worker thread
												// if the channel is full; chunks are dropped.
												let _ = chunk_tx.try_send(AppEvent::BangChunk(chunk));
											},
										);
										let result = tokio::select! {
											r = shell_fut => r,
											_ = cancel_rx => {
												// Cancelled by Ctrl+C — future is dropped,
												// which cleans up the shell process.
												Ok(rho_agent::tools::ToolOutput {
													content: String::new(),
													is_error: true,
												})
											}
										};
										let is_error = result.map_or(true, |o| o.is_error);
										let _ = done_tx.send(AppEvent::BangDone { is_error }).await;
									});
								},
								InputAction::UserMessage(text) => {
									// If already streaming, cancel the current agent run
									// first. cancel_streaming increments agent_generation
									// immediately, closing the race window where stale
									// events could corrupt the new agent's state.
									if matches!(mode, AppMode::Streaming) {
										if let Some(partial) = cancel_streaming(
											&mut mode, &mut app, &terminal,
											&mut agent_generation, &mut agent_cancel,
										) {
											session.append(Message::Assistant(partial)).await?;
										}
									}

									let user_msg = Message::User(UserMessage { content: text.to_owned() });
									app.chat.add_message(user_msg.clone());
									session.append(user_msg).await?;

									mode = AppMode::Streaming;
									app.chat.start_streaming();
									app.status.clear_work_status();
									app.status.start_working();
									app.update_status_border(terminal.columns());
									agent_cancel = Some(spawn_agent(
										&model,
										session.messages(),
										&tools,
										&system_prompt,
										&settings,
										&api_key,
										&tx,
										&mut agent_generation,
									));
								},
							}
						} else if data == "\x1b" && matches!(result, InputResult::Ignored) {
							// Esc in idle — editor didn't consume (no autocomplete).
							let text = app.editor.get_text();
							if !text.trim().is_empty() && text.trim_start().starts_with('!') {
								// Exit bash-mode input.
								app.editor.set_text("");
								app.sync_editor_border_color();
							}
						}
					}
				},
				rho_tui::TerminalEvent::Resize(cols, _) => {
					app.update_status_border(cols);
					app.tui.request_render_force();
				},
				rho_tui::TerminalEvent::Paste(ref text) => {
					// Always accept paste so the user can type ahead while streaming.
					let bracketed = format!("\x1b[200~{text}\x1b[201~");
					app.handle_input(&bracketed);
				},
			},
			Some(AppEvent::Agent(tagged)) => {
				// Drop events from old (stale) agent generations.
				if tagged.generation != agent_generation {
					continue;
				}
				match tagged.event {
					AgentEvent::TurnStart { .. } => {
						if matches!(mode, AppMode::Streaming) {
							app.status.set_working_phase("Thinking");
						}
					},
					AgentEvent::TextDelta(text) => {
						if matches!(mode, AppMode::Streaming) {
							app.chat.append_text(&text);
						}
					},
					AgentEvent::ThinkingDelta(text) => {
						if matches!(mode, AppMode::Streaming) {
							app.chat.append_thinking(&text);
						}
					},
					AgentEvent::ToolCallStart { name, .. } => {
						if matches!(mode, AppMode::Streaming) {
							app.status.set_working_phase(&generate_tool_phase(&name));
							app.chat.set_tool_executing(Some(name));
						}
					},
					AgentEvent::ToolCallResult { .. } => {
						app.status.set_working_phase("Thinking");
						app.chat.set_tool_executing(None);
					},
					AgentEvent::MessageComplete(message) => {
						if let Some(ref usage) = message.usage {
							app.status
								.set_usage(usage.input_tokens, usage.output_tokens);
							app.update_status_border(terminal.columns());
						}
						session.append(Message::Assistant(message.clone())).await?;
						app.chat.finish_streaming();
						app.chat.add_message(Message::Assistant(message));
					},
					AgentEvent::ToolResultComplete { tool_use_id, content, is_error } => {
						let tool_msg =
							Message::ToolResult(ToolResultMessage { tool_use_id, content, is_error });
						app.chat.add_message(tool_msg.clone());
						session.append(tool_msg).await?;
						// Start streaming for next LLM turn.
						app.chat.start_streaming();
					},
					AgentEvent::RetryScheduled { attempt, delay_ms, error } => {
						show_chat_message(
							&mut app,
							&format!("Retrying (attempt {attempt}) in {delay_ms}ms: {error}"),
						);
					},
					AgentEvent::Done(outcome) => {
						mode = AppMode::Idle;
						agent_cancel = None;
						app.chat.finish_streaming();
						app.status.finish_working();
						app.update_status_border(terminal.columns());

						// Auto-compaction only for non-cancelled outcomes.
						let maybe_usage = match &outcome {
							AgentOutcome::Stop { usage } | AgentOutcome::MaxTokens { usage } => {
								usage.as_ref()
							},
							_ => None, // Cancelled, Failed -> no usage
						};
						if let Some(usage) = maybe_usage {
							let context_tokens = usage.input_tokens
								+ usage.output_tokens
								+ usage.cache_creation_input_tokens
								+ usage.cache_read_input_tokens;
							let compaction_settings = &settings.compaction;
							if crate::compaction::settings::should_compact(
								context_tokens,
								model.context_window,
								compaction_settings,
							) {
								show_chat_message(&mut app, "Context nearing limit, compacting...");
								match crate::compaction::compact::run_compaction(
									&session,
									&model,
									&api_key,
									compaction_settings,
									None,
								)
								.await
								{
									Ok(result) => {
										match session.append_compaction(
											&result.summary,
											result.short_summary.as_deref(),
											&result.first_kept_entry_id,
											result.tokens_before,
											result.details,
										) {
											Ok(()) => {
												let msg = result
													.short_summary
													.as_deref()
													.unwrap_or("Conversation compacted.");
												show_chat_message(&mut app, &format!("Auto-compacted: {msg}"));
											},
											Err(e) => {
												show_chat_message(
													&mut app,
													&format!("Auto-compaction succeeded but failed to persist: {e}"),
												);
											},
										}
									},
									Err(e) => {
										show_chat_message(&mut app, &format!("Auto-compaction failed: {e}"));
									},
								}
							}
						}

						match outcome {
							AgentOutcome::Cancelled => {}, // Clean exit
							AgentOutcome::MaxTokens { .. } => {
								show_chat_message(
									&mut app,
									"Warning: response truncated (max tokens reached).",
								);
							},
							AgentOutcome::Failed { error } => {
								show_chat_message(&mut app, &format!("Error: {error}"));
							},
							_ => {},
						}
					},
				}
			},
			Some(AppEvent::BangChunk(chunk)) => {
				if matches!(mode, AppMode::BangRunning) {
					app.chat.append_bang_output(&chunk);
				}
			},
			Some(AppEvent::BangDone { is_error }) => {
				if matches!(mode, AppMode::BangRunning) {
					app.chat.finish_bang(is_error);
					mode = AppMode::Idle;
				}
			},
			None => break, // Channel closed.
		}

		// Re-render after every event.
		app.tui.request_render();
		app.render_to_tui(&mut terminal)?;
	}

	// Cleanup: move cursor past the rendered content and restore the terminal.
	app.tui.stop(&mut terminal)?;

	Ok(())
}

/// Convert a crossterm `KeyEvent` to a string suitable for rho-tui
/// component input handling.
///
/// Components expect raw terminal escape sequences (e.g., `"\x1b[A"` for up
/// arrow, `"\r"` for enter). This mirrors the `crossterm_key_to_string`
/// function in `rho_tui::terminal` but is reproduced here since the
/// version in rho-tui is not publicly exported.
fn crossterm_key_to_string(key: &crossterm::event::KeyEvent) -> String {
	use crossterm::event::{KeyCode, KeyModifiers};

	match key.code {
		KeyCode::Char(c) => {
			if key.modifiers.contains(KeyModifiers::CONTROL) {
				// Control characters: Ctrl+a => 0x01, Ctrl+c => 0x03, etc.
				if c.is_ascii_lowercase() {
					let ctrl = (c as u8) - b'a' + 1;
					return String::from(ctrl as char);
				}
			}
			if key.modifiers.contains(KeyModifiers::ALT) {
				return format!("\x1b{c}");
			}
			c.to_string()
		},
		KeyCode::Enter => "\r".to_owned(),
		KeyCode::Tab => "\t".to_owned(),
		KeyCode::BackTab => "\x1b[Z".to_owned(),
		KeyCode::Backspace => "\x7f".to_owned(),
		KeyCode::Esc => "\x1b".to_owned(),
		KeyCode::Up => "\x1b[A".to_owned(),
		KeyCode::Down => "\x1b[B".to_owned(),
		KeyCode::Right => "\x1b[C".to_owned(),
		KeyCode::Left => "\x1b[D".to_owned(),
		KeyCode::Home => "\x1b[H".to_owned(),
		KeyCode::End => "\x1b[F".to_owned(),
		KeyCode::PageUp => "\x1b[5~".to_owned(),
		KeyCode::PageDown => "\x1b[6~".to_owned(),
		KeyCode::Delete => "\x1b[3~".to_owned(),
		KeyCode::Insert => "\x1b[2~".to_owned(),
		KeyCode::F(n) => match n {
			1 => "\x1bOP".to_owned(),
			2 => "\x1bOQ".to_owned(),
			3 => "\x1bOR".to_owned(),
			4 => "\x1bOS".to_owned(),
			5 => "\x1b[15~".to_owned(),
			6 => "\x1b[17~".to_owned(),
			7 => "\x1b[18~".to_owned(),
			8 => "\x1b[19~".to_owned(),
			9 => "\x1b[20~".to_owned(),
			10 => "\x1b[21~".to_owned(),
			11 => "\x1b[23~".to_owned(),
			12 => "\x1b[24~".to_owned(),
			_ => String::new(),
		},
		_ => String::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_generate_tool_phase_known_tools() {
		assert_eq!(generate_tool_phase("read"), "Reading file");
		assert_eq!(generate_tool_phase("bash"), "Running bash");
		assert_eq!(generate_tool_phase("grep"), "Searching files");
		assert_eq!(generate_tool_phase("find"), "Finding files");
		assert_eq!(generate_tool_phase("glob"), "Finding files");
	}

	#[test]
	fn test_generate_tool_phase_unknown() {
		assert_eq!(generate_tool_phase("custom_tool"), "Running custom_tool");
	}

	#[test]
	fn test_crossterm_key_to_string_char() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Char('a'),
			crossterm::event::KeyModifiers::NONE,
		);
		assert_eq!(crossterm_key_to_string(&key), "a");
	}

	#[test]
	fn test_crossterm_key_to_string_ctrl_c() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Char('c'),
			crossterm::event::KeyModifiers::CONTROL,
		);
		assert_eq!(crossterm_key_to_string(&key), "\x03");
	}

	#[test]
	fn test_crossterm_key_to_string_enter() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Enter,
			crossterm::event::KeyModifiers::NONE,
		);
		assert_eq!(crossterm_key_to_string(&key), "\r");
	}

	#[test]
	fn test_crossterm_key_to_string_arrow_up() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Up,
			crossterm::event::KeyModifiers::NONE,
		);
		assert_eq!(crossterm_key_to_string(&key), "\x1b[A");
	}

	#[test]
	fn test_crossterm_key_to_string_alt_x() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Char('x'),
			crossterm::event::KeyModifiers::ALT,
		);
		assert_eq!(crossterm_key_to_string(&key), "\x1bx");
	}

	#[test]
	fn test_crossterm_key_to_string_backspace() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Backspace,
			crossterm::event::KeyModifiers::NONE,
		);
		assert_eq!(crossterm_key_to_string(&key), "\x7f");
	}

	#[test]
	fn test_crossterm_key_to_string_escape() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Esc,
			crossterm::event::KeyModifiers::NONE,
		);
		assert_eq!(crossterm_key_to_string(&key), "\x1b");
	}

	#[test]
	fn test_crossterm_key_to_string_delete() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Delete,
			crossterm::event::KeyModifiers::NONE,
		);
		assert_eq!(crossterm_key_to_string(&key), "\x1b[3~");
	}

	#[test]
	fn test_crossterm_key_to_string_f1() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::F(1),
			crossterm::event::KeyModifiers::NONE,
		);
		assert_eq!(crossterm_key_to_string(&key), "\x1bOP");
	}

	#[test]
	fn test_crossterm_key_to_string_unknown_returns_empty() {
		let key = crossterm::event::KeyEvent::new(
			crossterm::event::KeyCode::Null,
			crossterm::event::KeyModifiers::NONE,
		);
		assert_eq!(crossterm_key_to_string(&key), "");
	}
}
