//! This fork's own features, kept apart from the upstream crates so that merging upstream stays
//! easy. Upstream code calls in at a few hook points, each marked `// fork:`; FORK.md lists them.

pub mod references;

use std::sync::OnceLock;

pub use references::References;

static REFERENCES: OnceLock<References> = OnceLock::new();

/// Reads this fork's settings at boot and refuses invalid ones, as `Config::from_env` does.
pub fn init() -> anyhow::Result<()> {
    let references = references_from(|name| std::env::var(name).ok())?;
    REFERENCES.set(references).map_err(|_| anyhow::anyhow!("campfire_fork::init ran twice"))
}

/// `CAMPFIRE_REFERENCES`, from any variable lookup (tests pass a map); none when unset or blank.
fn references_from(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<References> {
    match get("CAMPFIRE_REFERENCES").filter(|v| !v.trim().is_empty()) {
        Some(json) => References::from_json(&json).map_err(|e| anyhow::anyhow!("CAMPFIRE_REFERENCES {e}")),
        None => Ok(References::default()),
    }
}

/// A text message's HTML with its references linked (`Presenter::content`), or as it is when
/// none are configured or `init` hasn't run (the upstream tests).
pub fn link_references(html: String) -> String {
    match REFERENCES.get().filter(|references| !references.is_empty()) {
        Some(references) => references.link(&html),
        None => html,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn references(value: Option<&str>) -> anyhow::Result<References> {
        references_from(|name| (name == "CAMPFIRE_REFERENCES").then(|| value.map(str::to_string)).flatten())
    }

    #[test]
    fn reads_reference_rules() {
        assert!(references(None).unwrap().is_empty());
        assert!(references(Some("  ")).unwrap().is_empty());
        let rules = r#"[{ "prefixes": ["DEV"], "url": "https://app.clickup.com/t/1/{id}" }]"#;
        let linked = references(Some(rules)).unwrap().link("DEV-1");
        assert!(linked.contains("https://app.clickup.com/t/1/DEV-1"), "{linked}");
        let error = references(Some("[1]")).unwrap_err();
        assert!(error.to_string().starts_with("CAMPFIRE_REFERENCES rule 1:"), "{error}");
    }

    #[test]
    fn leaves_html_alone_before_init() {
        assert_eq!(link_references("<p>DEV-1 !2</p>".into()), "<p>DEV-1 !2</p>");
    }
}
