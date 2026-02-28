use std::path::Path;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{Concurrency, OnToolUpdate, Tool, ToolOutput};

/// Tool that provides image information and resizing.
pub struct ImageTool;

#[async_trait]
impl Tool for ImageTool {
	fn name(&self) -> &'static str {
		"image"
	}

	fn description(&self) -> &'static str {
		include_str!("../prompts/tools/image.md")
	}

	fn input_schema(&self) -> Value {
		json!({
			 "type": "object",
			 "properties": {
				  "action": {
						"type": "string",
						"description": "Action to perform: \"info\" or \"resize\""
				  },
				  "path": {
						"type": "string",
						"description": "Path to the image file"
				  },
				  "width": {
						"type": "integer",
						"description": "Target width for resize"
				  },
				  "height": {
						"type": "integer",
						"description": "Target height for resize"
				  },
				  "output": {
						"type": "string",
						"description": "Output path for resized image (default: overwrite input)"
				  }
			 },
			 "required": ["action", "path"]
		})
	}

	fn concurrency(&self) -> Concurrency {
		Concurrency::Exclusive
	}

	async fn execute(&self, input: Value, cwd: &Path, _cancel: &CancellationToken, _on_update: Option<&OnToolUpdate>) -> anyhow::Result<ToolOutput> {
		let action = input
			.get("action")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: action"))?;

		let raw_path = input
			.get("path")
			.and_then(Value::as_str)
			.ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

		let image_path = cwd.join(raw_path);

		match action {
			"info" => {
				let bytes = match tokio::fs::read(&image_path).await {
					Ok(b) => b,
					Err(e) => {
						return Ok(ToolOutput {
							content:  format!("Error reading image {}: {e}", image_path.display()),
							is_error: true,
						});
					},
				};

				let result =
					tokio::task::spawn_blocking(move || rho_tools::image::PhotonImage::parse(&bytes))
						.await
						.map_err(|e| anyhow::anyhow!("Image info task panicked: {e}"))?;

				match result {
					Ok(img) => Ok(ToolOutput {
						content:  format!("Width: {}, Height: {}", img.width(), img.height()),
						is_error: false,
					}),
					Err(e) => {
						Ok(ToolOutput { content: format!("Image parse error: {e}"), is_error: true })
					},
				}
			},
			"resize" => {
				let width = input
					.get("width")
					.and_then(Value::as_u64)
					.ok_or_else(|| anyhow::anyhow!("Missing required parameter for resize: width"))?
					as u32;

				let height = input
					.get("height")
					.and_then(Value::as_u64)
					.ok_or_else(|| anyhow::anyhow!("Missing required parameter for resize: height"))?
					as u32;

				let output_path = input
					.get("output")
					.and_then(Value::as_str)
					.map_or_else(|| image_path.clone(), |p| cwd.join(p));

				let bytes = match tokio::fs::read(&image_path).await {
					Ok(b) => b,
					Err(e) => {
						return Ok(ToolOutput {
							content:  format!("Error reading image {}: {e}", image_path.display()),
							is_error: true,
						});
					},
				};

				let encoded = tokio::task::spawn_blocking(move || {
					let img = rho_tools::image::PhotonImage::parse(&bytes)?;
					let resized = img.resize(width, height, rho_tools::image::SamplingFilter::Lanczos3);
					resized.encode(0, 100) // PNG format
				})
				.await
				.map_err(|e| anyhow::anyhow!("Image resize task panicked: {e}"))?;

				match encoded {
					Ok(data) => {
						if let Err(e) = tokio::fs::write(&output_path, &data).await {
							return Ok(ToolOutput {
								content:  format!(
									"Error writing resized image to {}: {e}",
									output_path.display()
								),
								is_error: true,
							});
						}
						Ok(ToolOutput {
							content:  format!("Image resized and saved to {}.", output_path.display()),
							is_error: false,
						})
					},
					Err(e) => {
						Ok(ToolOutput { content: format!("Image resize error: {e}"), is_error: true })
					},
				}
			},
			_ => Ok(ToolOutput {
				content:  format!("Unknown action: {action}. Use \"info\" or \"resize\"."),
				is_error: true,
			}),
		}
	}
}

#[cfg(test)]
mod tests {
	use tokio_util::sync::CancellationToken;

	use super::*;

	#[tokio::test]
	async fn test_image_info_png() {
		// Create a minimal 1x1 PNG in memory
		let dir = tempfile::tempdir().unwrap();
		let img_path = dir.path().join("test.png");

		// Valid 1x1 white RGB PNG generated with correct CRCs
		let png_bytes: &[u8] = &[
			0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
			0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
			0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
			0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0x0d, 0xef, 0x46, 0xb8, 0x00, 0x00, 0x00,
			0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
		];
		std::fs::write(&img_path, png_bytes).unwrap();

		let tool = ImageTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(json!({"action": "info", "path": img_path.to_str().unwrap()}), Path::new("/"), &ct, None)
			.await
			.unwrap();
		assert!(!result.is_error, "Unexpected error: {}", result.content);
		assert!(result.content.contains("Width: 1"), "Expected 'Width: 1' in: {}", result.content);
		assert!(result.content.contains("Height: 1"), "Expected 'Height: 1' in: {}", result.content);
	}

	#[tokio::test]
	async fn test_image_unknown_action() {
		let tool = ImageTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(json!({"action": "invalid", "path": "/tmp/test.png"}), Path::new("/"), &ct, None)
			.await
			.unwrap();
		assert!(result.is_error);
		assert!(
			result.content.contains("Unknown action"),
			"Expected 'Unknown action' in: {}",
			result.content
		);
	}

	#[tokio::test]
	async fn test_image_missing_path() {
		let tool = ImageTool;
		let ct = CancellationToken::new();
		let result = tool
			.execute(json!({"action": "info"}), Path::new("/"), &ct, None)
			.await;
		assert!(result.is_err(), "Expected error for missing path parameter");
	}
}
