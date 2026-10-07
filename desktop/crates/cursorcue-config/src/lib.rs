use serde::{Deserialize, Serialize};
use std::{
    env, fmt, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_CONFIG_BYTES: u64 = 65_536;
static FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    Arrow,
    Dot,
    Circle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Smoothing {
    Off,
    Light,
    Medium,
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub modifiers: u32,
    pub key: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    pub cursor_scale: f32,
    pub opacity: f32,
    pub cursor_style: CursorStyle,
    pub animation_enabled: bool,
    pub animation_duration_ms: u32,
    pub smoothing: Smoothing,
    pub hotkeys: [Hotkey; 5],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            cursor_scale: 1.0,
            opacity: 1.0,
            cursor_style: CursorStyle::Arrow,
            animation_enabled: true,
            animation_duration_ms: 200,
            smoothing: Smoothing::Light,
            hotkeys: [0x46, 0x48, 0x52, 0x44, 0x47].map(|key| Hotkey { modifiers: 6, key }),
        }
    }
}

#[derive(Debug)]
pub struct LoadResult {
    pub settings: Settings,
    pub warning: Option<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Json(serde_json::Error),
    Validation(String),
    UnsupportedSchema(u64),
    MissingAppData,
    TooLarge,
    Cleanup {
        source: Box<ConfigError>,
        cleanup: io::Error,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "Configuration file error: {error}"),
            Self::Json(error) => write!(f, "Invalid configuration JSON: {error}"),
            Self::Validation(message) => write!(f, "Invalid configuration: {message}"),
            Self::UnsupportedSchema(version) => write!(
                f,
                "Configuration schema {version} is newer than supported schema {SCHEMA_VERSION}"
            ),
            Self::MissingAppData => write!(f, "APPDATA is missing or empty"),
            Self::TooLarge => write!(
                f,
                "Configuration exceeds {MAX_CONFIG_BYTES} bytes; file preserved"
            ),
            Self::Cleanup { source, cleanup } => {
                write!(f, "{source}; temporary file cleanup failed: {cleanup}")
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Cleanup { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for ConfigError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<serde_json::Error> for ConfigError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version > SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchema(u64::from(
                self.schema_version,
            )));
        }
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigError::Validation("schema_version must be 1".into()));
        }
        if !self.cursor_scale.is_finite() || !(0.5..=3.0).contains(&self.cursor_scale) {
            return Err(ConfigError::Validation(
                "cursor_scale must be between 0.5 and 3.0".into(),
            ));
        }
        if !self.opacity.is_finite() || !(0.2..=1.0).contains(&self.opacity) {
            return Err(ConfigError::Validation(
                "opacity must be between 0.2 and 1.0".into(),
            ));
        }
        if !(120..=450).contains(&self.animation_duration_ms) {
            return Err(ConfigError::Validation(
                "animation_duration_ms must be between 120 and 450".into(),
            ));
        }
        for (index, hotkey) in self.hotkeys.iter().enumerate() {
            if hotkey.modifiers == 0 && hotkey.key == 0 {
                continue;
            }
            if hotkey.modifiers == 0 || hotkey.modifiers & !0x0f != 0 {
                return Err(ConfigError::Validation(format!(
                    "Hotkey {} requires Alt, Control, Shift or Windows modifiers",
                    index + 1
                )));
            }
            if !matches!(hotkey.key, 0x30..=0x39 | 0x41..=0x5a | 0x70..=0x87) {
                return Err(ConfigError::Validation(format!(
                    "Hotkey {} requires A-Z, 0-9 or F1-F24",
                    index + 1
                )));
            }
            if self.hotkeys[..index].contains(hotkey) {
                return Err(ConfigError::Validation(format!(
                    "Hotkey {} duplicates another shortcut",
                    index + 1
                )));
            }
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<LoadResult, ConfigError> {
        let bytes = match read_config(path) {
            Ok(bytes) => bytes,
            Err(ConfigError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(LoadResult {
                    settings: Self::default(),
                    warning: None,
                });
            }
            Err(error) => return Err(error),
        };
        reject_future_schema(&bytes)?;
        let parsed = serde_json::from_slice::<Self>(&bytes)
            .map_err(ConfigError::from)
            .and_then(|settings| {
                settings.validate()?;
                Ok(settings)
            });
        match parsed {
            Ok(settings) => Ok(LoadResult {
                settings,
                warning: None,
            }),
            Err(error) => {
                let backup = write_backup(path, &bytes)?;
                let settings = Self::default();
                settings.save(path)?;
                Ok(LoadResult {
                    settings,
                    warning: Some(format!(
                        "{error}. Defaults restored; original file saved to {}",
                        backup.display()
                    )),
                })
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        match read_config(path) {
            Ok(bytes) => reject_future_schema(&bytes)?,
            Err(ConfigError::Io(error)) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        fs::create_dir_all(parent)?;
        let (temporary, mut file) = create_unique_file(path, "tmp")?;
        let result = file.write_all(&bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = result {
            return Err(cleanup_after_error(&temporary, error.into()));
        }
        if let Err(error) = fs::rename(&temporary, path) {
            return Err(cleanup_after_error(&temporary, error.into()));
        }
        Ok(())
    }
}

pub fn appdata_config_path() -> Result<PathBuf, ConfigError> {
    let appdata = env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::MissingAppData)?;
    Ok(config_path_from(Path::new(&appdata)))
}

fn config_path_from(appdata: &Path) -> PathBuf {
    appdata.join("CursorCue").join("config.json")
}

fn read_config(path: &Path) -> Result<Vec<u8>, ConfigError> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    Ok(bytes)
}

fn reject_future_schema(bytes: &[u8]) -> Result<(), ConfigError> {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes)
        && let Some(version) = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
        && version > u64::from(SCHEMA_VERSION)
    {
        return Err(ConfigError::UnsupportedSchema(version));
    }
    Ok(())
}

fn create_unique_file(path: &Path, extension: &str) -> Result<(PathBuf, fs::File), ConfigError> {
    for _ in 0..100 {
        let mut filename = path
            .file_name()
            .ok_or_else(|| ConfigError::Validation("configuration path must name a file".into()))?
            .to_os_string();
        let id = FILE_ID.fetch_add(1, Ordering::Relaxed);
        filename.push(format!(".{}-{id}.{extension}", std::process::id()));
        let candidate = path.with_file_name(filename);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(ConfigError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a unique configuration file",
    )))
}

fn write_backup(path: &Path, bytes: &[u8]) -> Result<PathBuf, ConfigError> {
    let (backup, mut file) = create_unique_file(path, "corrupt.bak")?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = result {
        return Err(cleanup_after_error(&backup, error.into()));
    }
    Ok(backup)
}

fn cleanup_after_error(path: &Path, source: ConfigError) -> ConfigError {
    match fs::remove_file(path) {
        Ok(()) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => source,
        Err(cleanup) => ConfigError::Cleanup {
            source: Box::new(source),
            cleanup,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn appdata_path_resolves_inside_cursorcue_directory() {
        assert_eq!(
            config_path_from(Path::new("task-profile")),
            Path::new("task-profile")
                .join("CursorCue")
                .join("config.json")
        );
    }
}
