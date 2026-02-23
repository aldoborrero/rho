use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "rho", version, about = "Interactive coding agent")]
pub struct Cli {
	/// Initial message to send to the AI
	#[arg(trailing_var_arg = true)]
	pub message: Vec<String>,

	/// Model name (e.g., claude-sonnet-4-5-20250929)
	#[arg(short, long, default_value = "claude-sonnet-4-5-20250929")]
	pub model: String,

	/// Continue most recent session
	#[arg(short, long)]
	pub r#continue: bool,

	/// Resume specific session by ID
	#[arg(short, long)]
	pub resume: Option<String>,

	/// Override system prompt
	#[arg(long)]
	pub system_prompt: Option<String>,

	/// Append to system prompt
	#[arg(long)]
	pub append_system_prompt: Option<String>,

	/// Anthropic API key (overrides env/config)
	#[arg(long)]
	pub api_key: Option<String>,

	/// Thinking level
	#[arg(long, default_value = "off")]
	pub thinking: String,

	/// Ephemeral session (no persistence)
	#[arg(long)]
	pub no_session: bool,

	/// Non-interactive print mode
	#[arg(short, long)]
	pub print: bool,
}

impl Cli {
	pub fn initial_message(&self) -> Option<String> {
		if self.message.is_empty() {
			None
		} else {
			Some(self.message.join(" "))
		}
	}
}
