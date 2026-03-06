use std::sync::Arc;

use crate::types::Message;

/// Outcome of a before-tool-call hook.
pub enum ToolCallAction {
	/// Proceed normally.
	Continue,
	/// Block execution; return this error message to the LLM.
	Block { reason: String },
	/// Proceed with modified input.
	ModifyInput { input: serde_json::Value },
}

/// Modification to a tool result (after execution).
pub struct ToolResultModification {
	pub content:  Option<String>,
	pub is_error: Option<bool>,
}

/// Modification to context (before LLM call).
pub struct ContextModification {
	/// Additional system prompt text to append.
	pub append_system_prompt: Option<String>,
	/// Messages to inject before the LLM call.
	pub inject_messages:      Vec<Message>,
}

/// Hook dispatch interface. All methods default to no-op.
/// Errors are fail-open (logged, never crash the agent).
#[async_trait::async_trait]
pub trait AgentHooks: Send + Sync {
	async fn before_tool_call(
		&self,
		name: &str,
		id: &str,
		input: &serde_json::Value,
	) -> anyhow::Result<ToolCallAction> {
		let _ = (name, id, input);
		Ok(ToolCallAction::Continue)
	}

	async fn after_tool_call(
		&self,
		name: &str,
		id: &str,
		content: &str,
		is_error: bool,
	) -> anyhow::Result<Option<ToolResultModification>> {
		let _ = (name, id, content, is_error);
		Ok(None)
	}

	async fn before_context(
		&self,
		messages: &[Message],
	) -> anyhow::Result<Option<ContextModification>> {
		let _ = messages;
		Ok(None)
	}

	async fn on_agent_event(&self, event: &crate::events::AgentEvent) {
		let _ = event;
	}
}

/// Composite dispatcher: fans out to multiple hook implementations.
///
/// - `before_tool_call`: first `Block` wins; `ModifyInput` composes (last
///   writer)
/// - `after_tool_call`: modifications compose (last writer per field)
/// - `before_context`: all `inject_messages` collected, `append_system_prompt`
///   concatenated
/// - `on_agent_event`: all observers called
pub struct HookChain {
	hooks: Vec<Arc<dyn AgentHooks>>,
}

impl HookChain {
	pub fn new(hooks: Vec<Arc<dyn AgentHooks>>) -> Self {
		Self { hooks }
	}
}

#[async_trait::async_trait]
impl AgentHooks for HookChain {
	async fn before_tool_call(
		&self,
		name: &str,
		id: &str,
		input: &serde_json::Value,
	) -> anyhow::Result<ToolCallAction> {
		let mut current_input: Option<serde_json::Value> = None;

		for hook in &self.hooks {
			let effective_input = current_input.as_ref().unwrap_or(input);
			match hook.before_tool_call(name, id, effective_input).await {
				Ok(ToolCallAction::Block { reason }) => {
					return Ok(ToolCallAction::Block { reason });
				},
				Ok(ToolCallAction::ModifyInput { input: modified }) => {
					current_input = Some(modified);
				},
				Ok(ToolCallAction::Continue) => {},
				Err(e) => {
					eprintln!("[hook] before_tool_call error (fail-open): {e}");
				},
			}
		}

		match current_input {
			Some(modified) => Ok(ToolCallAction::ModifyInput { input: modified }),
			None => Ok(ToolCallAction::Continue),
		}
	}

	async fn after_tool_call(
		&self,
		name: &str,
		id: &str,
		content: &str,
		is_error: bool,
	) -> anyhow::Result<Option<ToolResultModification>> {
		let mut merged: Option<ToolResultModification> = None;

		for hook in &self.hooks {
			match hook.after_tool_call(name, id, content, is_error).await {
				Ok(Some(modification)) => {
					merged = Some(match merged {
						Some(mut prev) => {
							if modification.content.is_some() {
								prev.content = modification.content;
							}
							if modification.is_error.is_some() {
								prev.is_error = modification.is_error;
							}
							prev
						},
						None => modification,
					});
				},
				Ok(None) => {},
				Err(e) => {
					eprintln!("[hook] after_tool_call error (fail-open): {e}");
				},
			}
		}

		Ok(merged)
	}

	async fn before_context(
		&self,
		messages: &[Message],
	) -> anyhow::Result<Option<ContextModification>> {
		let mut merged_prompt: Option<String> = None;
		let mut merged_messages: Vec<Message> = Vec::new();

		for hook in &self.hooks {
			match hook.before_context(messages).await {
				Ok(Some(modification)) => {
					if let Some(append) = modification.append_system_prompt {
						match &mut merged_prompt {
							Some(existing) => {
								existing.push('\n');
								existing.push_str(&append);
							},
							None => merged_prompt = Some(append),
						}
					}
					merged_messages.extend(modification.inject_messages);
				},
				Ok(None) => {},
				Err(e) => {
					eprintln!("[hook] before_context error (fail-open): {e}");
				},
			}
		}

		if merged_prompt.is_none() && merged_messages.is_empty() {
			Ok(None)
		} else {
			Ok(Some(ContextModification {
				append_system_prompt: merged_prompt,
				inject_messages:      merged_messages,
			}))
		}
	}

	async fn on_agent_event(&self, event: &crate::events::AgentEvent) {
		for hook in &self.hooks {
			hook.on_agent_event(event).await;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::UserMessage;

	/// A no-op hook for testing chain with no implementations.
	struct NoopHook;

	#[async_trait::async_trait]
	impl AgentHooks for NoopHook {}

	/// A hook that blocks a specific tool.
	struct BlockingHook {
		tool_name: &'static str,
		reason:    &'static str,
	}

	#[async_trait::async_trait]
	impl AgentHooks for BlockingHook {
		async fn before_tool_call(
			&self,
			name: &str,
			_id: &str,
			_input: &serde_json::Value,
		) -> anyhow::Result<ToolCallAction> {
			if name == self.tool_name {
				Ok(ToolCallAction::Block { reason: self.reason.to_owned() })
			} else {
				Ok(ToolCallAction::Continue)
			}
		}
	}

	/// A hook that modifies tool input by injecting a field.
	struct ModifyInputHook {
		key:   &'static str,
		value: &'static str,
	}

	#[async_trait::async_trait]
	impl AgentHooks for ModifyInputHook {
		async fn before_tool_call(
			&self,
			_name: &str,
			_id: &str,
			input: &serde_json::Value,
		) -> anyhow::Result<ToolCallAction> {
			let mut modified = input.clone();
			modified[self.key] = serde_json::Value::String(self.value.to_owned());
			Ok(ToolCallAction::ModifyInput { input: modified })
		}
	}

	/// A hook that modifies tool result content.
	struct ModifyResultHook {
		new_content: &'static str,
	}

	#[async_trait::async_trait]
	impl AgentHooks for ModifyResultHook {
		async fn after_tool_call(
			&self,
			_name: &str,
			_id: &str,
			_content: &str,
			_is_error: bool,
		) -> anyhow::Result<Option<ToolResultModification>> {
			Ok(Some(ToolResultModification {
				content:  Some(self.new_content.to_owned()),
				is_error: None,
			}))
		}
	}

	/// A hook that provides context.
	struct ContextHook {
		prompt:   &'static str,
		messages: Vec<Message>,
	}

	#[async_trait::async_trait]
	impl AgentHooks for ContextHook {
		async fn before_context(
			&self,
			_messages: &[Message],
		) -> anyhow::Result<Option<ContextModification>> {
			Ok(Some(ContextModification {
				append_system_prompt: Some(self.prompt.to_owned()),
				inject_messages:      self.messages.clone(),
			}))
		}
	}

	/// A hook that always errors.
	struct ErrorHook;

	#[async_trait::async_trait]
	impl AgentHooks for ErrorHook {
		async fn before_tool_call(
			&self,
			_name: &str,
			_id: &str,
			_input: &serde_json::Value,
		) -> anyhow::Result<ToolCallAction> {
			anyhow::bail!("intentional test error")
		}

		async fn after_tool_call(
			&self,
			_name: &str,
			_id: &str,
			_content: &str,
			_is_error: bool,
		) -> anyhow::Result<Option<ToolResultModification>> {
			anyhow::bail!("intentional test error")
		}

		async fn before_context(
			&self,
			_messages: &[Message],
		) -> anyhow::Result<Option<ContextModification>> {
			anyhow::bail!("intentional test error")
		}
	}

	#[tokio::test]
	async fn empty_chain_returns_continue() {
		let chain = HookChain::new(vec![]);
		let action = chain
			.before_tool_call("bash", "id_1", &serde_json::json!({}))
			.await
			.unwrap();
		assert!(matches!(action, ToolCallAction::Continue));
	}

	#[tokio::test]
	async fn empty_chain_returns_none_for_after_tool_call() {
		let chain = HookChain::new(vec![]);
		let result = chain
			.after_tool_call("bash", "id_1", "output", false)
			.await
			.unwrap();
		assert!(result.is_none());
	}

	#[tokio::test]
	async fn empty_chain_returns_none_for_before_context() {
		let chain = HookChain::new(vec![]);
		let result = chain.before_context(&[]).await.unwrap();
		assert!(result.is_none());
	}

	#[tokio::test]
	async fn blocking_hook_returns_block() {
		let chain = HookChain::new(vec![Arc::new(BlockingHook {
			tool_name: "bash",
			reason:    "not allowed",
		})]);
		let action = chain
			.before_tool_call("bash", "id_1", &serde_json::json!({}))
			.await
			.unwrap();
		match action {
			ToolCallAction::Block { reason } => assert_eq!(reason, "not allowed"),
			_ => panic!("expected Block"),
		}
	}

	#[tokio::test]
	async fn modify_then_block_returns_block() {
		let chain = HookChain::new(vec![
			Arc::new(ModifyInputHook { key: "injected", value: "true" }),
			Arc::new(BlockingHook { tool_name: "bash", reason: "blocked" }),
		]);
		let action = chain
			.before_tool_call("bash", "id_1", &serde_json::json!({}))
			.await
			.unwrap();
		match action {
			ToolCallAction::Block { reason } => assert_eq!(reason, "blocked"),
			_ => panic!("expected Block"),
		}
	}

	#[tokio::test]
	async fn modify_input_composes_last_writer() {
		let chain = HookChain::new(vec![
			Arc::new(ModifyInputHook { key: "a", value: "1" }),
			Arc::new(ModifyInputHook { key: "b", value: "2" }),
		]);
		let action = chain
			.before_tool_call("bash", "id_1", &serde_json::json!({}))
			.await
			.unwrap();
		match action {
			ToolCallAction::ModifyInput { input } => {
				assert_eq!(input["a"], "1");
				assert_eq!(input["b"], "2");
			},
			_ => panic!("expected ModifyInput"),
		}
	}

	#[tokio::test]
	async fn before_context_collects_all() {
		let chain = HookChain::new(vec![
			Arc::new(ContextHook {
				prompt:   "hook1 context",
				messages: vec![Message::User(UserMessage { content: "msg1".to_owned() })],
			}),
			Arc::new(ContextHook {
				prompt:   "hook2 context",
				messages: vec![Message::User(UserMessage { content: "msg2".to_owned() })],
			}),
		]);
		let result = chain.before_context(&[]).await.unwrap().unwrap();
		let prompt = result.append_system_prompt.unwrap();
		assert!(prompt.contains("hook1 context"));
		assert!(prompt.contains("hook2 context"));
		assert_eq!(result.inject_messages.len(), 2);
	}

	#[tokio::test]
	async fn error_in_hook_is_fail_open() {
		let chain = HookChain::new(vec![
			Arc::new(ErrorHook),
			Arc::new(BlockingHook { tool_name: "bash", reason: "blocked" }),
		]);
		// ErrorHook fails, but BlockingHook should still run.
		let action = chain
			.before_tool_call("bash", "id_1", &serde_json::json!({}))
			.await
			.unwrap();
		match action {
			ToolCallAction::Block { reason } => assert_eq!(reason, "blocked"),
			_ => panic!("expected Block after error hook was skipped"),
		}
	}

	#[tokio::test]
	async fn error_in_after_tool_call_is_fail_open() {
		let chain = HookChain::new(vec![
			Arc::new(ErrorHook),
			Arc::new(ModifyResultHook { new_content: "modified" }),
		]);
		let result = chain
			.after_tool_call("bash", "id_1", "original", false)
			.await
			.unwrap();
		let modification = result.unwrap();
		assert_eq!(modification.content.unwrap(), "modified");
	}

	#[tokio::test]
	async fn error_in_before_context_is_fail_open() {
		let chain = HookChain::new(vec![
			Arc::new(ErrorHook),
			Arc::new(ContextHook { prompt: "works", messages: vec![] }),
		]);
		let result = chain.before_context(&[]).await.unwrap().unwrap();
		assert_eq!(result.append_system_prompt.unwrap(), "works");
	}

	#[tokio::test]
	async fn noop_hook_in_chain_is_transparent() {
		let chain = HookChain::new(vec![Arc::new(NoopHook)]);
		let action = chain
			.before_tool_call("bash", "id_1", &serde_json::json!({}))
			.await
			.unwrap();
		assert!(matches!(action, ToolCallAction::Continue));

		let result = chain
			.after_tool_call("bash", "id_1", "output", false)
			.await
			.unwrap();
		assert!(result.is_none());

		let ctx = chain.before_context(&[]).await.unwrap();
		assert!(ctx.is_none());
	}
}
