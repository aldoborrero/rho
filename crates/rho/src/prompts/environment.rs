use super::types::EnvItem;
use crate::utils::platform;

pub fn gather() -> Vec<EnvItem> {
	let mut items = vec![EnvItem { label: "OS".into(), value: platform::os_name().into() }];

	if let Some(version) = platform::os_version() {
		items.push(EnvItem { label: "Distro".into(), value: version });
	}

	items.push(EnvItem { label: "Arch".into(), value: platform::arch().into() });

	let count = platform::cpu_count();
	let cpu_value = if let Some(model) = platform::cpu_model() {
		format!("{count}x {model}")
	} else {
		format!("{count} cores")
	};
	items.push(EnvItem { label: "CPU".into(), value: cpu_value });

	if let Some(term) = platform::terminal() {
		items.push(EnvItem { label: "Terminal".into(), value: term });
	}

	items
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gather_returns_os_and_arch() {
		let items = gather();
		let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
		assert!(labels.contains(&"OS"), "missing OS item");
		assert!(labels.contains(&"Arch"), "missing Arch item");
	}

	#[test]
	fn gather_values_are_nonempty() {
		let items = gather();
		for item in &items {
			assert!(!item.value.is_empty(), "empty value for: {}", item.label);
		}
	}

	#[test]
	fn gather_includes_cpu() {
		let items = gather();
		let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
		assert!(labels.contains(&"CPU"), "missing CPU item");
	}
}
