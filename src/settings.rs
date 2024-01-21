use std::default::Default;

use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;
use base64::engine::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

#[derive(Debug, Deserialize)]
pub(crate) struct Settings {
    pub(crate) discord: DiscordSettings,
    pub(crate) komga: KomgaSettings,
    pub(crate) newsletters: NewslettersSettings,
    pub(crate) stats: StatsSettings,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StatsSettings {
    pub(crate) enabled: bool,
    pub(crate) category: String,
    pub(crate) schedule: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct NewslettersSettings {
    pub(crate) enabled: bool,
    pub(crate) templates_dir: String,
    pub(crate) content_dir: String,
    pub(crate) url: String,
    pub(crate) channel: String,
    pub(crate) schedule: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiscordSettings {
    pub(crate) token: String,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct KomgaSettings {
    pub(crate) url: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) public_url: Option<String>,
    pub(crate) libraries: Option<Vec<String>>,
}

impl From<&KomgaSettings> for reqwest::Client {
    fn from(value: &KomgaSettings) -> Self {
        let basic_auth_header = format!("Basic {}", BASE64.encode(format!("{}:{}", &value.username, &value.password)));
        return reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::ACCEPT,
                    reqwest::header::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(basic_auth_header.as_str())
                        .expect("failed to create basic auth header"),
                );
                headers
            })
            .build()
            .expect("failed to build reqwest client");
    }
}

impl From<KomgaSettings> for komga::apis::configuration::Configuration {
    fn from(value: KomgaSettings) -> Self {
        Self {
            client: reqwest::Client::from(&value),
            base_path: value.url,
            basic_auth: Some((value.username, Some(value.password))),
            ..Default::default()
        }
    }
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let s = Config::builder()
            .add_source(File::with_name("config/default"))
            .add_source(File::with_name("config/local").required(false))
            .add_source(Environment::default())
            .build()?;
        s.try_deserialize()
    }
}