use std::path::PathBuf;

use clap::Parser;
use rho::{cli::Cli, config, modes, session::SessionManager, tools::create_default_registry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let cli = Cli::parse();
	let settings = rho::settings::load(&cli)?;
	let models_config = rho::models_config::load_models_config()?;
	let registry = rho::models_config::build_registry(&models_config, &settings);
	let resolved = rho::models_config::resolve_model(
		&settings.model.default,
		&registry,
		&settings,
		&models_config,
	)?;

	let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
	let session = if cli.no_session {
		SessionManager::in_memory()
	} else if cli.r#continue {
		SessionManager::continue_recent_in(&cwd, None)?
	} else if let Some(ref id) = cli.resume {
		// Try as file path first, then as session ID in the default dir.
		let path = PathBuf::from(id);
		if path.exists() {
			SessionManager::open(&path, None)?
		} else {
			let session_dir = config::get_default_session_dir(&cwd);
			SessionManager::resume(&session_dir.join(id))
		}
	} else {
		SessionManager::create(&cwd, None)?
	};

	let tools = create_default_registry();

	modes::interactive::run_interactive(
		&cli,
		settings,
		models_config,
		registry,
		resolved,
		session,
		tools,
	)
	.await
}
