use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::cli::timeout_from_seconds;
use crate::provider::configurable::{RestProfile, RestSettings};
use crate::provider::geoiplookup::DEFAULT_TIMEOUT;

const BUILTIN_PROFILE: &str = "geoiplookup";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    schema_version: u32,
    default_profile: Option<String>,
    timeout_seconds: Option<u64>,
    #[serde(default)]
    profiles: BTreeMap<String, ProfileConfig>,
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            default_profile: None,
            timeout_seconds: None,
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "adapter", deny_unknown_fields)]
enum ProfileConfig {
    #[serde(rename = "geoiplookup")]
    Geoiplookup { timeout_seconds: Option<u64> },
    #[serde(rename = "maxmind")]
    Maxmind {
        timeout_seconds: Option<u64>,
        service: MaxMindService,
        product: MaxMindProduct,
        locales: Option<Vec<String>>,
        auth: AuthConfig,
    },
    #[serde(rename = "rest-json")]
    RestJson(Box<RestProfile>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum AuthConfig {
    #[serde(rename = "basic")]
    Basic {
        username_env: String,
        password_env: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MaxMindService {
    Geoip,
    Geolite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MaxMindProduct {
    Country,
    City,
    Insights,
}

pub(crate) struct MaxMindSettings {
    pub service: MaxMindService,
    pub product: MaxMindProduct,
    pub locales: Vec<String>,
    pub account_id: String,
    pub license_key: String,
}

pub(crate) enum Adapter {
    Geoiplookup,
    Maxmind(MaxMindSettings),
    RestJson(Box<RestSettings>),
}

impl std::fmt::Debug for Adapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Geoiplookup => "Geoiplookup",
            Self::Maxmind(_) => "Maxmind(<redacted>)",
            Self::RestJson(_) => "RestJson(<redacted>)",
        })
    }
}

#[derive(Debug)]
pub(crate) struct SelectedProfile {
    pub name: String,
    pub adapter: Adapter,
    pub timeout: Duration,
}

pub(crate) struct Config {
    file: FileConfig,
}

impl Config {
    /// An absent implicit configuration is allowed; a missing explicit path is not.
    pub fn load(explicit: Option<&Path>, env_path: Option<OsString>) -> Result<Self> {
        let (path, required) = if let Some(path) = explicit {
            (Some(path.to_path_buf()), true)
        } else if let Some(path) = env_path {
            if path.is_empty() {
                bail!("GEOHUEO_CONFIG must name a file");
            }
            (Some(PathBuf::from(path)), true)
        } else {
            (default_config_path(), false)
        };

        let Some(path) = path else {
            return Ok(Self {
                file: FileConfig::default(),
            });
        };
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    file: FileConfig::default(),
                });
            }
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", path.display()));
            }
        };
        let file: FileConfig = toml::from_str(&text)
            .with_context(|| format!("invalid configuration in {}", path.display()))?;
        Self::from_file(file)
            .with_context(|| format!("invalid configuration in {}", path.display()))
    }

    fn from_file(file: FileConfig) -> Result<Self> {
        if file.schema_version != 1 {
            bail!(
                "unsupported schema_version {} (expected 1)",
                file.schema_version
            );
        }
        if let Some(seconds) = file.timeout_seconds {
            validate_timeout(seconds, "timeout_seconds")?;
        }
        for (name, profile) in &file.profiles {
            validate_profile_name(name)?;
            match profile {
                ProfileConfig::Geoiplookup { timeout_seconds } => {
                    if let Some(seconds) = timeout_seconds {
                        validate_timeout(*seconds, &format!("profiles.{name}.timeout_seconds"))?;
                    }
                }
                ProfileConfig::Maxmind {
                    timeout_seconds,
                    service,
                    product,
                    locales,
                    auth,
                } => {
                    if name == BUILTIN_PROFILE {
                        bail!("the built-in profile '{BUILTIN_PROFILE}' cannot be replaced");
                    }
                    if let Some(seconds) = timeout_seconds {
                        validate_timeout(*seconds, &format!("profiles.{name}.timeout_seconds"))?;
                    }
                    if *service == MaxMindService::Geolite && *product == MaxMindProduct::Insights {
                        bail!("profiles.{name}: GeoLite does not support Insights");
                    }
                    if let Some(locales) = locales {
                        if locales.is_empty()
                            || locales.iter().any(|locale| {
                                !matches!(
                                    locale.as_str(),
                                    "de" | "en" | "es" | "fr" | "ja" | "pt-BR" | "ru" | "zh-CN"
                                )
                            })
                        {
                            bail!(
                                "profiles.{name}.locales must contain supported MaxMind language codes"
                            );
                        }
                    }
                    let AuthConfig::Basic {
                        username_env,
                        password_env,
                    } = auth;
                    validate_env_name(username_env)?;
                    validate_env_name(password_env)?;
                }
                ProfileConfig::RestJson(rest) => {
                    if name == BUILTIN_PROFILE {
                        bail!("the built-in profile '{BUILTIN_PROFILE}' cannot be replaced");
                    }
                    if let Some(seconds) = rest.timeout_seconds {
                        validate_timeout(seconds, &format!("profiles.{name}.timeout_seconds"))?;
                    }
                    rest.validate()
                        .with_context(|| format!("profiles.{name}"))?;
                }
            }
        }
        if let Some(name) = &file.default_profile {
            validate_profile_name(name)?;
            if name != BUILTIN_PROFILE && !file.profiles.contains_key(name) {
                bail!("default_profile '{name}' is not configured");
            }
        }
        Ok(Self { file })
    }

    pub fn profile_names(&self) -> Vec<&str> {
        let mut names = self
            .file
            .profiles
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !self.file.profiles.contains_key(BUILTIN_PROFILE) {
            names.push(BUILTIN_PROFILE);
            names.sort_unstable();
        }
        names
    }

    pub fn select(
        &self,
        cli_profile: Option<&str>,
        env_profile: Option<OsString>,
        cli_timeout: Option<Duration>,
    ) -> Result<SelectedProfile> {
        let env_profile = if cli_profile.is_some() {
            None
        } else {
            env_profile
                .map(|value| {
                    value.into_string().map_err(|_| {
                        anyhow!("GEOHUEO_PROFILE must contain a valid UTF-8 profile name")
                    })
                })
                .transpose()?
        };
        let name = cli_profile
            .or(env_profile.as_deref())
            .or(self.file.default_profile.as_deref())
            .unwrap_or(BUILTIN_PROFILE);
        validate_profile_name(name)?;

        let profile = self.file.profiles.get(name);
        let profile_timeout = match profile {
            Some(ProfileConfig::Geoiplookup { timeout_seconds })
            | Some(ProfileConfig::Maxmind {
                timeout_seconds, ..
            }) => *timeout_seconds,
            Some(ProfileConfig::RestJson(rest)) => rest.timeout_seconds,
            None if name == BUILTIN_PROFILE => None,
            None => bail!("profile '{name}' is not configured"),
        };
        let timeout = cli_timeout
            .or_else(|| profile_timeout.and_then(timeout_from_seconds))
            .or_else(|| self.file.timeout_seconds.and_then(timeout_from_seconds))
            .unwrap_or(DEFAULT_TIMEOUT);

        let adapter = match profile {
            Some(ProfileConfig::Maxmind {
                service,
                product,
                locales,
                auth,
                ..
            }) => {
                let AuthConfig::Basic {
                    username_env,
                    password_env,
                } = auth;
                let account_id = read_secret(username_env)?;
                if !account_id.chars().all(|c| c.is_ascii_digit()) {
                    bail!(
                        "credential environment variable {username_env} must contain a numeric account ID"
                    );
                }
                Adapter::Maxmind(MaxMindSettings {
                    service: *service,
                    product: *product,
                    locales: locales.clone().unwrap_or_else(|| vec!["en".to_string()]),
                    account_id,
                    license_key: read_secret(password_env)?,
                })
            }
            Some(ProfileConfig::RestJson(rest)) => Adapter::RestJson(Box::new(rest.resolve()?)),
            _ => Adapter::Geoiplookup,
        };

        Ok(SelectedProfile {
            name: name.to_string(),
            adapter,
            timeout,
        })
    }
}

fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
        bail!("invalid profile name {name:?}");
    }
    Ok(())
}

fn validate_timeout(seconds: u64, field: &str) -> Result<()> {
    if timeout_from_seconds(seconds).is_none() {
        bail!("{field} must be a positive whole number of seconds within the supported range");
    }
    Ok(())
}

pub(crate) fn validate_env_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        bail!("invalid credential environment variable name {name:?}");
    }
    Ok(())
}

pub(crate) fn read_secret(name: &str) -> Result<String> {
    let value = env::var_os(name)
        .ok_or_else(|| anyhow!("credential environment variable {name} is not set"))?
        .into_string()
        .map_err(|_| anyhow!("credential environment variable {name} is not valid UTF-8"))?;
    if value.is_empty() || value.chars().any(char::is_control) {
        bail!(
            "credential environment variable {name} must be non-empty without control characters"
        );
    }
    Ok(value)
}

fn default_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var_os("APPDATA")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join("geohueo").join("config.toml"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "macos")]
        let fallback = Path::new("Library/Application Support");
        #[cfg(not(target_os = "macos"))]
        let fallback = Path::new(".config");

        if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
            let xdg = PathBuf::from(xdg);
            if xdg.is_absolute() {
                return Some(xdg.join("geohueo").join("config.toml"));
            }
        }
        env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join(fallback)
                .join("geohueo")
                .join("config.toml")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<Config> {
        Config::from_file(toml::from_str(input)?)
    }

    #[test]
    fn builtin_profile_is_available_without_configuration() {
        let config = Config::from_file(FileConfig::default()).unwrap();
        assert_eq!(config.profile_names(), ["geoiplookup"]);
        let selected = config.select(None, None, None).unwrap();
        assert_eq!(selected.name, "geoiplookup");
        assert!(matches!(selected.adapter, Adapter::Geoiplookup));
        assert_eq!(selected.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn alias_and_timeout_precedence() {
        let config = parse(
            "schema_version = 1\ndefault_profile = 'personal'\ntimeout_seconds = 7\n\
             [profiles.personal]\nadapter = 'geoiplookup'\ntimeout_seconds = 4\n",
        )
        .unwrap();
        assert_eq!(config.profile_names(), ["geoiplookup", "personal"]);
        assert_eq!(
            config.select(None, None, None).unwrap().timeout,
            Duration::from_secs(4)
        );
        assert_eq!(
            config
                .select(None, Some(OsString::from("geoiplookup")), None)
                .unwrap()
                .timeout,
            Duration::from_secs(7)
        );
        let selected = config
            .select(
                Some("personal"),
                Some(OsString::from("geoiplookup")),
                Some(Duration::from_secs(2)),
            )
            .unwrap();
        assert_eq!(selected.name, "personal");
        assert_eq!(selected.timeout, Duration::from_secs(2));
    }

    #[test]
    fn rejects_bad_schema_profiles_and_timeouts() {
        for input in [
            "schema_version = 2",
            "schema_version = 1\nunknown = 'value'",
            "schema_version = 1\ndefault_profile = 'missing'",
            "schema_version = 1\ntimeout_seconds = 0",
            "schema_version = 1\n[profiles.test]\nadapter = 'unknown'",
            "schema_version = 1\n[profiles.test]\nadapter = 'geoiplookup'\ntimeout_seconds = 0",
            "schema_version = 1\n[profiles.test]\nadapter = 'geoiplookup'\nunknown = 1",
            "schema_version = 1\n[profiles.maxmind]\nadapter = 'maxmind'\nservice = 'geolite'\nproduct = 'insights'\n[profiles.maxmind.auth]\ntype = 'basic'\nusername_env = 'MM_ACCOUNT'\npassword_env = 'MM_KEY'",
            "schema_version = 1\n[profiles.maxmind]\nadapter = 'maxmind'\nservice = 'geoip'\nproduct = 'city'\nlocales = ['xx']\n[profiles.maxmind.auth]\ntype = 'basic'\nusername_env = 'MM_ACCOUNT'\npassword_env = 'MM_KEY'",
            "schema_version = 1\n[profiles.maxmind]\nadapter = 'maxmind'\nservice = 'geoip'\nproduct = 'city'\n[profiles.maxmind.auth]\ntype = 'basic'\nusername_env = 'BAD=NAME'\npassword_env = 'MM_KEY'",
            "schema_version = 1\n[profiles.geoiplookup]\nadapter = 'maxmind'\nservice = 'geoip'\nproduct = 'city'\n[profiles.geoiplookup.auth]\ntype = 'basic'\nusername_env = 'MM_ACCOUNT'\npassword_env = 'MM_KEY'",
        ] {
            assert!(parse(input).is_err(), "unexpectedly accepted {input}");
        }
    }

    #[test]
    fn maxmind_profile_is_parsed_without_loading_unselected_secrets() {
        let config = parse(
            "schema_version = 1\n[profiles.mm]\nadapter = 'maxmind'\nservice = 'geoip'\nproduct = 'city'\nlocales = ['ru', 'en']\n[profiles.mm.auth]\ntype = 'basic'\nusername_env = 'MM_UNUSED_ACCOUNT'\npassword_env = 'MM_UNUSED_KEY'",
        )
        .unwrap();
        assert_eq!(config.profile_names(), ["geoiplookup", "mm"]);
        assert!(matches!(
            config.select(None, None, None).unwrap().adapter,
            Adapter::Geoiplookup
        ));
        assert!(config.select(Some("mm"), None, None).is_err());
    }

    #[test]
    fn rest_json_profile_validates_all_settings_but_only_reads_selected_secret() {
        let sample = "schema_version = 1\n[profiles.custom]\nadapter = 'rest-json'\nbase_url = 'https://geo.example.invalid'\nsupports_ipv4 = true\nsupports_ipv6 = false\n[profiles.custom.auth]\ntype = 'bearer'\ntoken_env = 'GEOHUEO_UNUSED_REST_TOKEN'\n[profiles.custom.request]\nmethod = 'GET'\npath = '/lookup/{ip}'\n[profiles.custom.response]\nrequired_fields = ['ip']\n[profiles.custom.response.fields]\nip = '/data/address'";
        let config = parse(sample).unwrap();
        assert_eq!(config.profile_names(), ["custom", "geoiplookup"]);
        assert!(matches!(
            config.select(None, None, None).unwrap().adapter,
            Adapter::Geoiplookup
        ));
        assert!(config.select(Some("custom"), None, None).is_err());
        for bad in [
            sample.replace("https://geo.example.invalid", "http://geo.example.invalid"),
            sample.replace("/lookup/{ip}", "/lookup/{ip}?token=secret"),
            format!("{sample}\nunknown = 1"),
            sample.replace("supports_ipv4 = true", "supports_ipv4 = false"),
        ] {
            assert!(parse(&bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn unknown_selected_profile_is_an_error() {
        let config = Config::from_file(FileConfig::default()).unwrap();
        assert!(
            config
                .select(Some("missing"), None, None)
                .unwrap_err()
                .to_string()
                .contains("not configured")
        );
        assert!(config.select(None, Some(OsString::from("")), None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn explicit_profile_overrides_non_utf8_environment_value() {
        use std::os::unix::ffi::OsStringExt;

        let config = Config::from_file(FileConfig::default()).unwrap();
        let invalid = OsString::from_vec(vec![0xff]);
        assert!(config.select(None, Some(invalid.clone()), None).is_err());
        assert!(
            config
                .select(Some(BUILTIN_PROFILE), Some(invalid), None)
                .is_ok()
        );
    }
}
