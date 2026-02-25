use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "rho", version, about = "Interactive coding agent")]
pub struct Cli {
	/// Initial message to send to the AI
	#[arg(trailing_var_arg = true)]
	pub message: Vec<String>,

	/// Model name or role (e.g., claude-sonnet-4-5-20250929, default, smol, slow)
	#[arg(short, long)]
	pub model: Option<String>,

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

	/// Thinking level (off, low, medium, high)
	#[arg(long)]
	pub thinking: Option<String>,

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
