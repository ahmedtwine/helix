use std::fs;
use std::io::Error as IoError;

use helix_loader::merge_toml_values;
use helix_loader::workspace_trust::{TrustQuery, WorkspaceTrust};
use helix_term::config::{Config, ConfigLoadError};

pub const DEFAULTS: &str = include_str!("../config/default.toml");

pub fn merge_with_user(user: Option<&str>) -> Result<String, ConfigLoadError> {
    let defaults: toml::Value = toml::from_str(DEFAULTS).map_err(ConfigLoadError::BadConfig)?;

    let merged = match user.map(str::trim).filter(|user| !user.is_empty()) {
        Some(user) => {
            let user: toml::Value = toml::from_str(user).map_err(ConfigLoadError::BadConfig)?;
            merge_toml_values(defaults, user, 3)
        }
        None => defaults,
    };

    Ok(toml::to_string(&merged).expect("merged config is serializable"))
}

pub fn load() -> Result<Config, ConfigLoadError> {
    let user = fs::read_to_string(helix_loader::config_file()).ok();
    let global = merge_with_user(user.as_deref())?;

    let local =
        fs::read_to_string(helix_loader::workspace_config_file()).map_err(ConfigLoadError::Error);

    let placeholder = ConfigLoadError::Error(IoError::other("no local config"));
    let global_parsed = Config::load(Ok(&global), Err(placeholder))?;

    let trust = WorkspaceTrust::new((&global_parsed.editor.workspace_trust).into());
    if trust.query_current(TrustQuery::LocalConfig).is_trusted() {
        let mut config = Config::load(Ok(&global), local)?;
        config.editor.workspace_trust = global_parsed.editor.workspace_trust;
        Ok(config)
    } else {
        Ok(global_parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_view::document::Mode;

    fn config_from(user: Option<&str>) -> Config {
        let merged = merge_with_user(user).unwrap();
        let placeholder = ConfigLoadError::Error(IoError::other("none"));
        Config::load(Ok(&merged), Err(placeholder)).unwrap()
    }

    #[test]
    fn defaults_ship_without_user_config() {
        let config = config_from(None);
        assert_eq!(
            config.theme.as_ref().map(|theme| theme.choose(None)),
            Some("ayu_dark")
        );
        assert!(config.editor.cursorline);
        assert!(config.editor.soft_wrap.enable.unwrap());
    }

    #[test]
    fn empty_user_config_keeps_defaults() {
        assert_eq!(config_from(Some("   \n\n  ")).theme, config_from(None).theme);
    }

    #[test]
    fn user_config_overrides_defaults() {
        let config = config_from(Some("theme = \"onedark\"\n[editor]\ncursorline = false\n"));
        assert_eq!(
            config.theme.as_ref().map(|theme| theme.choose(None)),
            Some("onedark")
        );
        assert!(!config.editor.cursorline);
        assert!(config.editor.mouse);
    }

    #[test]
    fn default_keybindings_are_present() {
        let config = config_from(None);
        assert!(config.keys.contains_key(&Mode::Normal));
    }
}
