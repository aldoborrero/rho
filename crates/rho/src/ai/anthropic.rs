/// Create a `rho_ai::Model` for the Anthropic provider from config.
pub fn create_model(config: &crate::config::Config, model_id: &str) -> rho_ai::Model {
	rho_ai::Model {
		id:              model_id.to_owned(),
		name:            model_id.to_owned(),
		provider:        "anthropic".to_owned(),
		api:             rho_ai::Api::AnthropicMessages,
		base_url:        config.base_url.clone(),
		reasoning:       false,
		supports_images: true,
		context_window:  200_000,
		max_tokens:      8192,
		cost:            rho_ai::ModelCost::default(),
	}
}
