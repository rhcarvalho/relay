use std::path::{Path, PathBuf};
use std::io::Write;
use std::fs;
use std::io;
use std::env;

use url_serde;
use serde_yaml;
use url::Url;
use smith_aorta::{generate_agent_id, generate_key_pair, AgentId, PublicKey, SecretKey};

/// Indicates config related errors.
#[derive(Fail, Debug)]
pub enum ConfigError {
    /// Failed to open the file.
    #[fail(display = "could not open config file")]
    CouldNotOpen(#[cause] io::Error),
    /// Failed to save a file.
    #[fail(display = "could not save a config file")]
    CouldNotSave(#[cause] io::Error),
    /// Parsing a YAML error failed.
    #[fail(display = "could not parse yaml file")]
    BadYaml(#[cause] serde_yaml::Error),
}

/// Agent specific configuration values.
#[derive(Serialize, Deserialize, Debug)]
#[serde(default)]
struct Agent {
    secret_key: Option<SecretKey>,
    public_key: Option<PublicKey>,
    id: Option<AgentId>,
    #[serde(with = "url_serde")] upstream: Url,
}

impl Default for Agent {
    fn default() -> Agent {
        Agent {
            secret_key: None,
            public_key: None,
            id: None,
            upstream: Url::parse("https://ingest.sentry.io/").unwrap(),
        }
    }
}

/// Config struct.
#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    #[serde(skip, default)] changed: bool,
    #[serde(skip, default = "PathBuf::new")] filename: PathBuf,
    #[serde(default)] agent: Agent,
}

impl Config {
    /// Loads a config from a given path.
    ///
    /// This can load a config that does not have any credentials yet in
    /// which case some methods will fail (like `secret_key`).  This can
    /// be verified with `is_configured`.  The `open` method handles
    /// this automatically.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Config, ConfigError> {
        let f = fs::File::open(&path).map_err(ConfigError::CouldNotOpen)?;
        let mut rv: Config =
            serde_yaml::from_reader(io::BufReader::new(f)).map_err(ConfigError::BadYaml)?;
        rv.filename = path.as_ref().to_path_buf();
        Ok(rv)
    }

    /// Loads a config from a path or initializes it.
    ///
    /// If the config does not exist or a secret key is not set, then credentials
    /// are regenerated automatically.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Config, ConfigError> {
        let path = match path.as_ref().canonicalize() {
            Ok(pathbuf) => pathbuf,
            Err(_) => env::current_dir()
                .map_err(ConfigError::CouldNotOpen)?
                .join(path.as_ref()),
        };
        let mut config = if fs::metadata(&path).is_ok() {
            Config::from_path(&path)?
        } else {
            Config {
                filename: path,
                changed: false,
                agent: Default::default(),
            }
        };
        if !config.is_configured() {
            config.regenerate_credentials();
        }
        Ok(config)
    }

    /// Writes back a config to the config file if the config changed.
    pub fn save(&mut self) -> Result<bool, ConfigError> {
        if !self.changed {
            return Ok(false);
        }
        let mut f = fs::File::create(&self.filename).map_err(ConfigError::CouldNotSave)?;
        serde_yaml::to_writer(&mut f, &self).map_err(ConfigError::BadYaml)?;
        f.write_all(b"\n").ok();
        self.changed = false;
        Ok(true)
    }

    /// Returns the filename of the config file.
    pub fn filename(&self) -> &Path {
        &self.filename
    }

    /// Regenerates the agent credentials.
    pub fn regenerate_credentials(&mut self) {
        let (sk, pk) = generate_key_pair();
        self.agent.secret_key = Some(sk);
        self.agent.public_key = Some(pk);
        self.agent.id = Some(generate_agent_id());
        self.changed = true;
    }

    /// Returns `true` if the config changed.
    pub fn changed(&self) -> bool {
        self.changed
    }

    /// Returns `true` if the config is ready to use.
    pub fn is_configured(&self) -> bool {
        self.agent.secret_key.is_some() && self.agent.public_key.is_some()
            && self.agent.id.is_some()
    }

    /// Returns the secret key if set.
    pub fn secret_key(&self) -> &SecretKey {
        self.agent.secret_key.as_ref().unwrap()
    }

    /// Returns the public key if set.
    pub fn public_key(&self) -> &PublicKey {
        self.agent.public_key.as_ref().unwrap()
    }

    /// Returns the agent ID.
    pub fn agent_id(&self) -> &AgentId {
        self.agent.id.as_ref().unwrap()
    }

    /// Returns the upstream target.
    pub fn upstream_target(&self) -> &Url {
        &self.agent.upstream
    }
}
