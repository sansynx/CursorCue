use cursorcue_config::{ConfigError, CursorStyle, Hotkey, Settings, Smoothing};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static TEST_ID: AtomicU64 = AtomicU64::new(0);
struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        let root = target.join("config-tests").join(format!(
            "{}-{}",
            std::process::id(),
            TEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("create task-owned test directory");
        Self(root)
    }
    fn config(&self) -> PathBuf {
        self.0.join("config.json")
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("clean task-owned test directory");
    }
}

#[test]
fn rejects_out_of_range_or_nonfinite_values() {
    for scale in [0.49, 3.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(
            Settings {
                cursor_scale: scale,
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
    }
    for opacity in [0.19, 1.01, f32::NAN, f32::INFINITY] {
        assert!(
            Settings {
                opacity,
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
    }
    for duration in [0, 119, 451, u32::MAX] {
        assert!(
            Settings {
                animation_duration_ms: duration,
                ..Settings::default()
            }
            .validate()
            .is_err()
        );
    }
    for (cursor_scale, opacity, animation_duration_ms) in [(0.5, 0.2, 120), (3.0, 1.0, 450)] {
        assert!(
            Settings {
                cursor_scale,
                opacity,
                animation_duration_ms,
                ..Settings::default()
            }
            .validate()
            .is_ok()
        );
    }
}

#[test]
fn validates_hotkey_keys_modifiers_duplicates_and_disabled_entries() {
    let mut settings = Settings::default();
    for key in [0x30, 0x39, 0x41, 0x5a, 0x70, 0x87] {
        settings.hotkeys[0] = Hotkey { modifiers: 1, key };
        assert!(settings.validate().is_ok());
    }
    for (modifiers, key) in [
        (0, 0x46),
        (6, 0),
        (16, 0x46),
        (16390, 0x46),
        (6, 0x61),
        (6, 0x88),
    ] {
        settings.hotkeys[0] = Hotkey { modifiers, key };
        assert!(settings.validate().is_err());
    }
    settings.hotkeys[0] = settings.hotkeys[1];
    assert!(settings.validate().is_err());
    settings.hotkeys = [Hotkey {
        modifiers: 0,
        key: 0,
    }; 5];
    assert!(settings.validate().is_ok());
}

#[test]
fn missing_file_returns_valid_defaults_without_warning() {
    let dir = TestDirectory::new();
    let result = Settings::load(&dir.config()).expect("load absent config");
    assert_eq!(result.settings, Settings::default());
    assert!(result.settings.validate().is_ok());
    assert!(result.warning.is_none());
}

#[test]
fn saves_roundtrips_and_replaces_existing_config() {
    let dir = TestDirectory::new();
    let path = dir.config();
    Settings::default().save(&path).expect("save defaults");
    let settings = Settings {
        cursor_scale: 2.75,
        opacity: 0.35,
        cursor_style: CursorStyle::Circle,
        animation_enabled: false,
        animation_duration_ms: 430,
        smoothing: Smoothing::Strong,
        hotkeys: [Hotkey {
            modifiers: 0,
            key: 0,
        }; 5],
        ..Settings::default()
    };
    settings.save(&path).expect("replace config");
    let loaded = Settings::load(&path).expect("reload config");
    assert_eq!(loaded.settings, settings);
    assert!(loaded.warning.is_none());
    assert_eq!(fs::read_dir(&dir.0).expect("list files").count(), 1);
}

#[test]
fn invalid_save_preserves_existing_file() {
    let dir = TestDirectory::new();
    let path = dir.config();
    let original = b"{\"opacity\":0.5}";
    fs::write(&path, original).expect("create old config");
    let settings = Settings {
        opacity: 0.0,
        ..Settings::default()
    };
    assert!(settings.save(&path).is_err());
    assert_eq!(fs::read(&path).expect("read old config"), original);
    assert_eq!(fs::read_dir(&dir.0).expect("list files").count(), 1);
}

#[test]
fn failed_replace_removes_temporary_file() {
    let dir = TestDirectory::new();
    let path = dir.config();
    fs::create_dir(&path).expect("create conflicting directory");
    assert!(Settings::default().save(&path).is_err());
    assert!(path.is_dir());
    assert_eq!(fs::read_dir(&dir.0).expect("list files").count(), 1);
}

#[test]
fn corrupt_config_is_backed_up_exactly_and_restored() {
    let dir = TestDirectory::new();
    let path = dir.config();
    let original = b"{ broken json\n\x00\xff";
    fs::write(&path, original).expect("create corrupt config");
    let loaded = Settings::load(&path).expect("recover corruption");
    assert_eq!(loaded.settings, Settings::default());
    assert!(loaded.warning.is_some());
    let entries: Vec<_> = fs::read_dir(&dir.0)
        .expect("list files")
        .map(|entry| entry.expect("entry").path())
        .filter(|entry| entry != &path)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(fs::read(&entries[0]).expect("read backup"), original);
    assert_eq!(
        Settings::load(&path)
            .expect("read repaired config")
            .settings,
        Settings::default()
    );
}

#[test]
fn missing_fields_default_but_malformed_fields_recover_with_warning() {
    let dir = TestDirectory::new();
    let path = dir.config();
    fs::write(&path, br#"{"opacity":0.5}"#).expect("create partial config");
    let loaded = Settings::load(&path).expect("load partial config");
    assert_eq!(
        loaded.settings,
        Settings {
            opacity: 0.5,
            ..Settings::default()
        }
    );
    assert!(loaded.warning.is_none());
    for original in [
        br#"{"cursor_scale":"big"}"#.as_slice(),
        br#"{"hotkeys":[]}"#,
        br#"{"cursor_style":"unknown"}"#,
        br#"{"opacity":0.0}"#,
    ] {
        fs::write(&path, original).expect("create malformed config");
        let loaded = Settings::load(&path).expect("recover malformed config");
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.warning.is_some());
    }
}

#[test]
fn future_schema_is_preserved_on_load_and_save() {
    let dir = TestDirectory::new();
    let path = dir.config();
    for version in [2, 4294967296u64] {
        let original = format!("{{\"schema_version\":{version},\"cursor_style\":\"future\"}}");
        fs::write(&path, &original).expect("create future config");
        assert!(
            matches!(Settings::load(&path), Err(ConfigError::UnsupportedSchema(found)) if found == version)
        );
        assert!(
            matches!(Settings::default().save(&path), Err(ConfigError::UnsupportedSchema(found)) if found == version)
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read preserved config"),
            original
        );
        assert_eq!(fs::read_dir(&dir.0).expect("list files").count(), 1);
    }
}

#[test]
fn nested_save_creates_parent_directory() {
    let dir = TestDirectory::new();
    let path = dir.0.join("nested/config.json");
    Settings::default().save(&path).expect("save nested config");
    assert_eq!(
        Settings::load(&path).expect("load nested config").settings,
        Settings::default()
    );
}

#[test]
fn oversized_config_is_preserved_on_load_and_save() {
    let dir = TestDirectory::new();
    let path = dir.config();
    let original = vec![b' '; 65_537];
    fs::write(&path, &original).expect("create oversized config");
    assert!(Settings::load(&path).is_err());
    assert!(Settings::default().save(&path).is_err());
    assert_eq!(fs::read(&path).expect("read preserved config"), original);
    assert_eq!(fs::read_dir(&dir.0).expect("list files").count(), 1);
}

#[cfg(windows)]
#[test]
fn failed_atomic_rename_preserves_original_and_cleans_temporary_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = TestDirectory::new();
    let path = dir.config();
    let original = br#"{"opacity":0.5}"#;
    fs::write(&path, original).expect("create config");
    let locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .expect("lock deletion while allowing read");
    let result = Settings::default().save(&path);
    drop(locked);
    assert!(result.is_err());
    assert_eq!(fs::read(&path).expect("read preserved config"), original);
    assert_eq!(fs::read_dir(&dir.0).expect("list files").count(), 1);
}
