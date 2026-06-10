use std::time::Duration;

use super::frame::{build_frame_message, build_frame_payload, FramePacer, FRAME_HEADER_LEN};
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

    let payload = build_frame_payload(3, 2, &image, RectRegion::new(1, 0, 2, 2)).unwrap();

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
fn build_frame_message_packs_header_and_rgba_bytes() {
    let image = vec![
        1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255, 13, 14, 15, 255, 16, 17, 18, 255,
    ];

    let message = build_frame_message(3, 2, &image, RectRegion::new(1, 0, 2, 2)).unwrap();

    assert_eq!(u16::from_le_bytes([message[0], message[1]]), 3);
    assert_eq!(u16::from_le_bytes([message[2], message[3]]), 2);
    assert_eq!(u16::from_le_bytes([message[4], message[5]]), 1);
    assert_eq!(u16::from_le_bytes([message[6], message[7]]), 0);
    assert_eq!(u16::from_le_bytes([message[8], message[9]]), 2);
    assert_eq!(u16::from_le_bytes([message[10], message[11]]), 2);
    assert_eq!(
        u32::from_le_bytes([message[12], message[13], message[14], message[15]]),
        16
    );
    assert_eq!(
        &message[FRAME_HEADER_LEN..],
        &[4, 5, 6, 255, 7, 8, 9, 255, 13, 14, 15, 255, 16, 17, 18, 255]
    );
}

#[test]
fn rect_region_union_keeps_combined_dirty_bounds() {
    let a = RectRegion::new(10, 20, 30, 40);
    let b = RectRegion::new(25, 10, 50, 20);

    let union = a.union(b);

    assert_eq!(union.x, 10);
    assert_eq!(union.y, 10);
    assert_eq!(union.width, 65);
    assert_eq!(union.height, 50);
}

#[test]
fn rect_region_union_saturates_oversized_bounds() {
    let a = RectRegion::new(100, 100, u16::MAX, u16::MAX);
    let b = RectRegion::new(0, 0, 1, 1);

    let union = a.union(b);

    assert_eq!(union.x, 0);
    assert_eq!(union.y, 0);
    assert_eq!(union.width, u16::MAX);
    assert_eq!(union.height, u16::MAX);
}

#[test]
fn frame_pacer_sends_first_update_then_coalesces_until_deadline() {
    let start = tokio::time::Instant::now();
    let mut pacer = FramePacer::new(Duration::from_millis(16));

    let first = pacer.queue(RectRegion::new(0, 0, 8, 8), start);

    let first = first.unwrap();
    assert_eq!(first.region, RectRegion::new(0, 0, 8, 8));
    assert_eq!(first.coalesced_updates, 0);
    assert_eq!(pacer.next_deadline(), None);

    let second = pacer.queue(
        RectRegion::new(8, 0, 8, 8),
        start + Duration::from_millis(5),
    );
    let third = pacer.queue(
        RectRegion::new(0, 8, 8, 8),
        start + Duration::from_millis(10),
    );

    assert!(second.is_none());
    assert!(third.is_none());
    assert_eq!(
        pacer.next_deadline(),
        Some(start + Duration::from_millis(16))
    );
    assert!(pacer.flush_due(start + Duration::from_millis(15)).is_none());
    let flushed = pacer.flush_due(start + Duration::from_millis(16)).unwrap();
    assert_eq!(flushed.region, RectRegion::new(0, 0, 16, 16));
    assert_eq!(flushed.coalesced_updates, 2);
    assert_eq!(pacer.next_deadline(), None);
}

#[test]
fn pointer_button_mapping_uses_web_button_order() {
    assert_eq!(mouse_button_from_web(0).unwrap(), RdpMouseButton::Left);
    assert_eq!(mouse_button_from_web(1).unwrap(), RdpMouseButton::Middle);
    assert_eq!(mouse_button_from_web(2).unwrap(), RdpMouseButton::Right);
    assert!(mouse_button_from_web(9).is_err());
}
