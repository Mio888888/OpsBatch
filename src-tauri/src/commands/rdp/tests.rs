use super::frame::build_frame_payload;
use super::input::mouse_button_from_web;
use super::types::{RdpMouseButton, RectRegion};
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

    let options = normalize_rdp_options(&request, "10.0.0.5", None).unwrap();

    assert_eq!(options.host, "10.0.0.5");
    assert_eq!(options.port, 3389);
    assert_eq!(options.width, 1280);
    assert_eq!(options.height, 720);
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
    assert!(normalize_rdp_options(&request, "10.0.0.5", Some(3390)).is_ok());
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
    let options = normalize_rdp_options(&request, "10.0.0.5", Some(3389)).unwrap();
    let credentials = RdpCredentials {
        username: "Administrator".to_string(),
        password: "secret".to_string(),
    };

    let config = build_ironrdp_config(&options, &credentials).unwrap();

    assert!(config.enable_tls);
    assert!(config.enable_credssp);
    assert!(config.autologon);
}

#[test]
fn build_frame_payload_copies_only_requested_region() {
    let image = vec![
        1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 255, 16, 17, 18, 255,
    ];

    let payload = build_frame_payload("s1", 3, 2, &image, RectRegion::new(1, 0, 2, 2)).unwrap();

    assert_eq!(payload.session_id, "s1");
    assert_eq!(payload.width, 3);
    assert_eq!(payload.height, 2);
    assert_eq!(payload.x, 1);
    assert_eq!(payload.y, 0);
    assert_eq!(payload.region_width, 2);
    assert_eq!(payload.region_height, 2);
    assert_eq!(
        payload.rgba,
        vec![4, 5, 6, 255, 7, 8, 9, 255, 13, 14, 15, 255, 16, 17, 18, 255]
    );
}

#[test]
fn pointer_button_mapping_uses_web_button_order() {
    assert_eq!(mouse_button_from_web(0).unwrap(), RdpMouseButton::Left);
    assert_eq!(mouse_button_from_web(1).unwrap(), RdpMouseButton::Middle);
    assert_eq!(mouse_button_from_web(2).unwrap(), RdpMouseButton::Right);
    assert!(mouse_button_from_web(9).is_err());
}
