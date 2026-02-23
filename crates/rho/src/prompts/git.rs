use std::{path::Path, time::Duration};

use tokio::process::Command;

use super::types::GitContext;

/// Timeout for most git commands.
const GIT_TIMEOUT: Duration = Duration::from_millis(1500);

/// Slightly longer timeout for `git status`.
const GIT_STATUS_TIMEOUT: Duration = Duration::from_secs(2);

async fn run_git(cwd: &Path, args: &[&str], timeout: Duration) -> Option<String> {
	let child = Command::new("git")
		.args(args)
		.current_dir(cwd)
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::null())
		.spawn()
		.ok()?;

	let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

	match result {
		Ok(Ok(output)) if output.status.success() => {
			let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
			Some(text)
		},
		_ => None,
	}
}

/// Gather git context for the system prompt.
///
/// Returns `None` if not in a git repository or if git commands fail.
pub async fn gather(cwd: &Path) -> Option<GitContext> {
	// Check if inside a git repo.
	let is_git = run_git(cwd, &["rev-parse", "--is-inside-work-tree"], GIT_TIMEOUT).await?;
	if is_git != "true" {
		return None;
	}

	// Get current branch.
	let current_branch = run_git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"], GIT_TIMEOUT).await?;

	// Detect main branch name.
	let main_branch = if run_git(cwd, &["rev-parse", "--verify", "main"], GIT_TIMEOUT)
		.await
		.is_some()
	{
		"main".to_owned()
	} else if run_git(cwd, &["rev-parse", "--verify", "master"], GIT_TIMEOUT)
		.await
		.is_some()
	{
		"master".to_owned()
	} else {
		"main".to_owned()
	};

	// Run status and log in parallel.
	let (status, commits) = tokio::join!(
		run_git(cwd, &["status", "--porcelain", "--untracked-files=no"], GIT_STATUS_TIMEOUT),
		run_git(cwd, &["log", "--oneline", "-5"], GIT_TIMEOUT),
	);

	let status = match status {
		Some(s) if s.is_empty() => "(clean)".to_owned(),
		Some(s) => s,
		None => "(status unavailable)".to_owned(),
	};

	let commits = match commits {
		Some(c) if !c.is_empty() => c,
		_ => "(no commits)".to_owned(),
	};

	Some(GitContext { is_repo: true, current_branch, main_branch, status, commits })
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Create a temporary git repository with an initial commit on a `main`
	/// branch.
	async fn temp_git_repo() -> tempfile::TempDir {
		let dir = tempfile::tempdir().unwrap();
		let p = dir.path();

		// Initialize, configure user, create initial commit.
		run_git(p, &["init", "-b", "main"], GIT_TIMEOUT)
			.await
			.expect("git init failed");
		run_git(p, &["config", "user.email", "test@test.com"], GIT_TIMEOUT)
			.await
			.expect("git config email failed");
		run_git(p, &["config", "user.name", "Test"], GIT_TIMEOUT)
			.await
			.expect("git config name failed");

		// Create a file and commit it so HEAD exists.
		std::fs::write(p.join("README.md"), "# test\n").unwrap();
		run_git(p, &["add", "README.md"], GIT_TIMEOUT)
			.await
			.expect("git add failed");
		run_git(p, &["commit", "-m", "initial commit"], GIT_TIMEOUT)
			.await
			.expect("git commit failed");

		dir
	}

	#[tokio::test]
	async fn gather_returns_some_in_git_repo() {
		let dir = temp_git_repo().await;
		let ctx = gather(dir.path()).await;
		assert!(ctx.is_some(), "expected Some in a git repo");
		let ctx = ctx.unwrap();
		assert!(ctx.is_repo);
		assert!(!ctx.current_branch.is_empty());
		assert_eq!(ctx.current_branch, "main");
		assert_eq!(ctx.main_branch, "main");
		assert_eq!(ctx.status, "(clean)");
		assert!(ctx.commits.contains("initial commit"));
	}

	#[tokio::test]
	async fn gather_returns_none_for_non_repo() {
		let dir = tempfile::tempdir().unwrap();
		let ctx = gather(dir.path()).await;
		assert!(ctx.is_none(), "expected None for non-git dir");
	}

	#[tokio::test]
	async fn run_git_returns_none_on_bad_command() {
		let dir = tempfile::tempdir().unwrap();
		let result = run_git(dir.path(), &["not-a-real-command"], GIT_TIMEOUT).await;
		assert!(result.is_none());
	}

	#[tokio::test]
	async fn run_git_returns_trimmed_output() {
		let dir = temp_git_repo().await;
		let result = run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"], GIT_TIMEOUT).await;
		assert_eq!(result, Some("true".to_owned()));
	}
}
