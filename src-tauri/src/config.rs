//! Settings and on-disk locations.

use std::{error::Error, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const DEFAULT_PORT: u16 = 47800;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Beta,
    Release,
}

impl Channel {
    pub fn url(self) -> &'static str {
        match self {
            Channel::Beta => "https://beta.enka.network/endfield/aic/",
            Channel::Release => "https://enkad.enka.network/",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Channel::Beta => "beta",
            Channel::Release => "release",
        }
    }

    /// Release is a different app build with no simulation and no debug handles;
    /// it has no adapter yet.
    pub fn supported(self) -> bool {
        self == Channel::Beta
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    port: u16,
    token: String,
    channel: Channel,
}

pub struct Config {
    pub port: u16,
    pub token: String,
    pub channel: Channel,
    data_dir: PathBuf,
}

impl Config {
    /// WebView2 user data folder: the planner's saves live here, apart from any browser.
    pub fn profile_dir(&self) -> PathBuf {
        self.data_dir.join("profile")
    }

    /// Saved plan states, kept outside the webview so they survive anything done in it.
    pub fn snapshots_dir(&self) -> PathBuf {
        self.data_dir.join("snapshots")
    }
}

pub fn load(app: &AppHandle) -> Result<Config, Box<dyn Error>> {
    let data_dir = app.path().app_local_data_dir()?;
    fs::create_dir_all(&data_dir)?;
    let path = data_dir.join("config.json");

    let stored = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok());
    let stored = match stored {
        Some(stored) => stored,
        None => {
            let stored = Stored {
                port: DEFAULT_PORT,
                token: new_token(),
                channel: Channel::Beta,
            };
            fs::write(&path, serde_json::to_string_pretty(&stored)?)?;
            stored
        }
    };

    let channel = if stored.channel.supported() {
        stored.channel
    } else {
        Channel::Beta
    };
    Ok(Config {
        port: stored.port,
        token: stored.token,
        channel,
        data_dir,
    })
}

fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
