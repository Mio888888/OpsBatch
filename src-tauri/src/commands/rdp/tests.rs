mod frame;

use ironrdp::pdu::rdp::client_info::{CompressionType, PerformanceFlags};

use super::input::mouse_button_from_web;
use super::protocol::{build_client_connector, rdp_static_channel_names};
use super::types::{RdpConnectionOptions, RdpMouseButton};
use super::*;
use crate::commands::rdp::config::build_ironrdp_config;

#[test]
fn normalize_options_defaults_to_rdp_port_and_safe_desktop_size() {
    let request = RdpConnectRequest {
        host_id: "host-1".to_string(),
        session_id: None,
        width: None,
        height: None,
        domain: None,
    };

    let options =
        normalize_rdp_options(&request, "10.0.0.5", None, &StoredRdpSettings::default()).unwrap();

    assert_eq!(options.host, "10.0.0.5");
    assert_eq!(options.port, 3389);
    assert_eq!(options.width, 1280);
    assert_eq!(options.height, 720);
    assert!(options.enable_clipboard);
    assert!(!options.enable_audio);
}

#[test]
fn normalize_options_rejects_empty_credentials() {
    let request = RdpConnectRequest {
        host_id: "host-1".to_string(),
        session_id: None,
        width: Some(1024),
        height: Some(768),
        domain: None,
    };

    let err = normalize_credentials("admin", None).unwrap_err();

    assert!(err.contains("RDP 密码不能为空"));
    assert!(normalize_rdp_options(
        &request,
        "10.0.0.5",
        Some(3390),
        &StoredRdpSettings::default()
    )
    .is_ok());
}

#[test]
fn normalize_options_prefers_saved_rdp_settings() {
    let request = RdpConnectRequest {
        host_id: "host-1".to_string(),
        session_id: None,
        width: Some(1024),
        height: Some(768),
        domain: Some("IGNORED".to_string()),
    };
    let settings = StoredRdpSettings {
        domain: Some("CORP".to_string()),
        desktop_width: Some(1920),
        desktop_height: Some(1080),
        enable_clipboard: Some(true),
        enable_audio: Some(true),
        map_disk: Some(true),
        disk_path: Some("/Users/admin/Downloads".to_string()),
    };

    let options = normalize_rdp_options(&request, "10.0.0.5", Some(3389), &settings).unwrap();

    assert_eq!(options.width, 1920);
    assert_eq!(options.height, 1080);
    assert_eq!(options.domain.as_deref(), Some("CORP"));
    assert!(options.enable_clipboard);
    assert!(options.enable_audio);
}

#[test]
fn stored_rdp_settings_accepts_clipboard_audio_and_disk_preferences() {
    let settings: StoredRdpSettings = serde_json::from_str(
        r#"{
            "enableClipboard": true,
            "enableAudio": true,
            "mapDisk": true,
            "diskPath": "/Users/admin/Downloads"
        }"#,
    )
    .unwrap();

    assert_eq!(settings.enable_clipboard, Some(true));
    assert_eq!(settings.enable_audio, Some(true));
    assert_eq!(settings.map_disk, Some(true));
    assert_eq!(
        settings.disk_path.as_deref(),
        Some("/Users/admin/Downloads")
    );
}

#[test]
fn rdp_config_advertises_tls_fallback_and_autologon() {
    let request = RdpConnectRequest {
        host_id: "host-1".to_string(),
        session_id: Some("session-1".to_string()),
        width: Some(1024),
        height: Some(768),
        domain: None,
    };
    let options = normalize_rdp_options(
        &request,
        "10.0.0.5",
        Some(3389),
        &StoredRdpSettings::default(),
    )
    .unwrap();
    let credentials = RdpCredentials {
        username: "Administrator".to_string(),
        password: "secret".to_string(),
    };

    let config = build_ironrdp_config(&options, &credentials).unwrap();

    assert!(config.enable_tls);
    assert!(config.enable_credssp);
    assert!(config.autologon);
    assert!(!config.enable_audio_playback);
    assert_eq!(config.compression_type, Some(CompressionType::Rdp61));
    assert!(config
        .performance_flags
        .contains(PerformanceFlags::DISABLE_WALLPAPER));
    assert!(config
        .performance_flags
        .contains(PerformanceFlags::DISABLE_FULLWINDOWDRAG));
    assert!(config
        .performance_flags
        .contains(PerformanceFlags::DISABLE_MENUANIMATIONS));
    assert!(config
        .performance_flags
        .contains(PerformanceFlags::DISABLE_THEMING));
    assert!(config
        .performance_flags
        .contains(PerformanceFlags::DISABLE_CURSOR_SHADOW));
    assert!(config
        .performance_flags
        .contains(PerformanceFlags::DISABLE_CURSORSETTINGS));
}

#[test]
fn rdp_config_enables_audio_playback_when_requested() {
    let request = RdpConnectRequest {
        host_id: "host-1".to_string(),
        session_id: Some("session-1".to_string()),
        width: Some(1024),
        height: Some(768),
        domain: None,
    };
    let settings = StoredRdpSettings {
        enable_audio: Some(true),
        ..StoredRdpSettings::default()
    };
    let options = normalize_rdp_options(&request, "10.0.0.5", Some(3389), &settings).unwrap();
    let credentials = RdpCredentials {
        username: "Administrator".to_string(),
        password: "secret".to_string(),
    };

    let config = build_ironrdp_config(&options, &credentials).unwrap();

    assert!(config.enable_audio_playback);
}

#[test]
fn rdp_connector_attaches_enabled_clipboard_and_audio_channels() {
    let options = test_rdp_options(true, true);
    let credentials = test_rdp_credentials();
    let config = build_ironrdp_config(&options, &credentials).unwrap();
    let client_addr = "127.0.0.1:50000".parse().unwrap();

    let connector = build_client_connector(config, client_addr, &options).unwrap();
    let channels = rdp_static_channel_names(&connector);

    assert_eq!(channels, vec!["cliprdr".to_string(), "rdpsnd".to_string()]);
}

#[test]
fn rdp_connector_skips_disabled_clipboard_and_audio_channels() {
    let options = test_rdp_options(false, false);
    let credentials = test_rdp_credentials();
    let config = build_ironrdp_config(&options, &credentials).unwrap();
    let client_addr = "127.0.0.1:50000".parse().unwrap();

    let connector = build_client_connector(config, client_addr, &options).unwrap();

    assert!(rdp_static_channel_names(&connector).is_empty());
}

#[test]
fn pointer_button_mapping_uses_web_button_order() {
    assert_eq!(mouse_button_from_web(0).unwrap(), RdpMouseButton::Left);
    assert_eq!(mouse_button_from_web(1).unwrap(), RdpMouseButton::Middle);
    assert_eq!(mouse_button_from_web(2).unwrap(), RdpMouseButton::Right);
    assert!(mouse_button_from_web(9).is_err());
}

fn test_rdp_options(enable_clipboard: bool, enable_audio: bool) -> RdpConnectionOptions {
    RdpConnectionOptions {
        host_id: "host-1".to_string(),
        session_id: "session-1".to_string(),
        host: "10.0.0.5".to_string(),
        port: 3389,
        width: 1024,
        height: 768,
        domain: None,
        enable_clipboard,
        enable_audio,
    }
}

fn test_rdp_credentials() -> RdpCredentials {
    RdpCredentials {
        username: "Administrator".to_string(),
        password: "secret".to_string(),
    }
}
