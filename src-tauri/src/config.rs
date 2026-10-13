//! Settings and on-disk locations.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

const DEFAULT_PORT: u16 = 47800;
const CONFIG_FILE: &str = "config.json";

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
    /// Whether the terminal pane was showing when the app last ran.
    #[serde(default)]
    terminal_visible: bool,
    /// Its width in physical pixels; 0 until it has been shown once.
    #[serde(default)]
    terminal_width: u32,
}

pub struct Config {
    pub port: u16,
    pub token: String,
    pub channel: Channel,
    pub terminal_visible: bool,
    pub terminal_width: u32,
    pub data_dir: PathBuf,
}

impl Config {
    pub fn mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    /// WebView2 user data folder: the planner's saves live here, apart from any browser.
    pub fn profile_dir(&self) -> PathBuf {
        self.data_dir.join("profile")
    }

    /// Saved plan states, kept outside the webview so they survive anything done in it.
    pub fn snapshots_dir(&self) -> PathBuf {
        self.data_dir.join("snapshots")
    }

    /// Folder the terminal pane starts in.
    pub fn workspace_dir(&self) -> PathBuf {
        self.data_dir.join("workspace")
    }

    /// Server entry in the form MCP clients read from a JSON config file.
    pub fn mcp_servers_json(&self) -> Value {
        json!({
            "mcpServers": { "enkad": {
                "type": "http",
                "url": self.mcp_url(),
                "headers": { "Authorization": format!("Bearer {}", self.token) },
            } }
        })
    }

    /// Make the terminal's folder ready for an agent: Claude Code and opencode find
    /// the MCP server through their own config file there, and the planner notes
    /// through the guide files.
    pub fn prepare_workspace(&self) -> Result<(), Box<dyn Error>> {
        let dir = self.workspace_dir();
        fs::create_dir_all(&dir)?;
        // The `enkad` entry is set on every start, since the port or token may have
        // changed. Anything else in these files is the user's and is kept.
        let headers = json!({ "Authorization": format!("Bearer {}", self.token) });
        set_json(
            &dir.join(".mcp.json"),
            &["mcpServers", "enkad"],
            json!({ "type": "http", "url": self.mcp_url(), "headers": headers }),
        )?;
        set_json(
            &dir.join("opencode.json"),
            &["mcp", "servers", "enkad"],
            json!({ "type": "remote", "url": self.mcp_url(), "headers": headers }),
        )?;
        // The guides are the user's to edit, so only missing ones are written.
        for name in ["CLAUDE.md", "AGENTS.md"] {
            let path = dir.join(name);
            if !path.exists() {
                fs::write(path, include_str!("../workspace-guide.md"))?;
            }
        }
        Ok(())
    }
}

/// Set to run a second copy against its own folder (profile, port, token), so
/// tests never touch the plans in the real one.
pub const DATA_DIR_OVERRIDE: &str = "ENKAD_MCP_DATA_DIR";

pub fn load(app: &AppHandle) -> Result<Config, Box<dyn Error>> {
    let data_dir = match std::env::var_os(DATA_DIR_OVERRIDE) {
        Some(dir) => PathBuf::from(dir),
        None => app.path().app_local_data_dir()?,
    };
    fs::create_dir_all(&data_dir)?;
    let path = data_dir.join(CONFIG_FILE);

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
                terminal_visible: false,
                terminal_width: 0,
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
        terminal_visible: stored.terminal_visible,
        terminal_width: stored.terminal_width,
        data_dir,
    })
}

/// Remember how the terminal pane was left. Other settings in the file are kept.
pub fn save_terminal(data_dir: &Path, visible: bool, width: u32) {
    let path = data_dir.join(CONFIG_FILE);
    let stored = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok());
    if let Some(mut stored) = stored {
        stored.terminal_visible = visible;
        stored.terminal_width = width;
        if let Ok(text) = serde_json::to_string_pretty(&stored) {
            let _ = fs::write(path, text);
        }
    }
}

/// Set one entry in a JSON file, keeping whatever else the file holds.
fn set_json(path: &Path, keys: &[&str], value: Value) -> Result<(), Box<dyn Error>> {
    let mut root = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let (last, parents) = keys.split_last().ok_or("no key given")?;
    let mut node = &mut root;
    for key in parents {
        let child = &mut node[*key];
        if !child.is_object() {
            *child = json!({});
        }
        node = child;
    }
    node[*last] = value;
    fs::write(path, serde_json::to_string_pretty(&root)?)?;
    Ok(())
}

fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
