mod frame;

use ironrdp::connector::{Sequence as _, Written};
use ironrdp::core::{encode_vec, Decode as _, ReadCursor, WriteBuf};
use ironrdp::dvc::pdu::{CreateRequestPdu, CreationStatus, DrdynvcClientPdu, DrdynvcServerPdu};
use ironrdp::dvc::{DrdynvcClient, DvcProcessor as _};
use ironrdp::graphics::zgfx::{compress_and_wrap_egfx, CompressionMode, Compressor};
use ironrdp::pdu::decode_cursor;
use ironrdp::pdu::gcc::ClientEarlyCapabilityFlags;
use ironrdp::pdu::geometry::ExclusiveRectangle;
use ironrdp::pdu::nego::ResponseFlags;
use ironrdp::pdu::rdp::client_info::{CompressionType, PerformanceFlags};
use ironrdp::pdu::x224::{X224Data, X224};
use ironrdp::pdu::{mcs, nego};
use ironrdp::svc::SvcProcessor as _;
use ironrdp_egfx::client::GraphicsPipelineHandler as _;
use ironrdp_egfx::pdu::{
    CapabilitiesConfirmPdu, CapabilitiesV107Flags, CapabilitiesV81Flags, CapabilitiesV8Flags,
    CapabilitySet, Codec1Type, CreateSurfacePdu, GfxPdu, MapSurfaceToOutputPdu,
    PixelFormat as EgfxPixelFormat, WireToSurface1Pdu,
};
use tokio::sync::mpsc;

use super::dynamic_channels::WINDOWS_H264_DIRECT_DVC_NAMES;
use super::egfx::{egfx_capability_diagnostics, RdpEgfxBridge};
use super::input::mouse_button_from_web;
use super::protocol::{
    build_client_connector, build_h264_direct_drdynvc_for_tests,
    h264_direct_encoded_video_receiver_count_for_tests, negotiation_response_flags_diagnostics,
    rdp_static_channel_names,
};
use super::types::{
    RdpConnectionOptions, RdpMouseButton, RdpStatusDetail, RdpTransportMode,
};
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
        transport_mode: None,
    };

    let options =
        normalize_rdp_options(&request, "10.0.0.5", None, &StoredRdpSettings::default()).unwrap();

    assert_eq!(options.host, "10.0.0.5");
    assert_eq!(options.port, 3389);
    assert_eq!(options.width, 1280);
    assert_eq!(options.height, 720);
    assert!(options.enable_clipboard);
    assert!(!options.enable_audio);
    assert_eq!(options.transport_mode, RdpTransportMode::LegacyBitmap);
}

#[test]
fn h264_direct_mode_reports_available_when_egfx_bridge_is_enabled() {
    let request = RdpConnectRequest {
        host_id: "host-1".to_string(),
        session_id: Some("rdp-test".to_string()),
        width: Some(1280),
        height: Some(720),
        domain: None,
        transport_mode: Some(RdpTransportMode::H264Direct),
    };

    let options =
        normalize_rdp_options(&request, "10.0.0.5", None, &StoredRdpSettings::default()).unwrap();

    assert_eq!(options.transport_mode, RdpTransportMode::H264Direct);
}

#[test]
fn h264_direct_mode_enables_audio_unless_saved_setting_disables_it() {
    let request = RdpConnectRequest {
        host_id: "host-1".to_string(),
        session_id: Some("rdp-test".to_string()),
        width: Some(1280),
        height: Some(720),
        domain: None,
        transport_mode: Some(RdpTransportMode::H264Direct),
    };

    let default_options =
        normalize_rdp_options(&request, "10.0.0.5", None, &StoredRdpSettings::default()).unwrap();
    assert!(default_options.enable_audio);

    let disabled_settings = StoredRdpSettings {
        enable_audio: Some(false),
        ..StoredRdpSettings::default()
    };
    let disabled_options =
        normalize_rdp_options(&request, "10.0.0.5", None, &disabled_settings).unwrap();
    assert!(!disabled_options.enable_audio);
}

#[test]
fn normalize_options_rejects_empty_credentials() {
    let request = RdpConnectRequest {
        host_id: "host-1".to_string(),
        session_id: None,
        width: Some(1024),
        height: Some(768),
        domain: None,
        transport_mode: None,
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
        transport_mode: None,
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
        transport_mode: None,
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
        transport_mode: None,
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
fn h264_direct_config_requests_rdpgfx_and_video_optimizations() {
    let mut options = test_rdp_options(true, true);
    options.transport_mode = RdpTransportMode::H264Direct;
    let credentials = test_rdp_credentials();

    let config = build_ironrdp_config(&options, &credentials).unwrap();

    assert!(config.enable_graphics_pipeline);
    assert!(config
        .client_early_capability_flags
        .contains(ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL));
    assert!(!config.disable_video_optimizations);
}

#[test]
fn h264_direct_connect_initial_advertises_rdpgfx_early_capability() {
    let mut options = test_rdp_options(true, true);
    options.transport_mode = RdpTransportMode::H264Direct;
    let credentials = test_rdp_credentials();
    let config = build_ironrdp_config(&options, &credentials).unwrap();
    let client_addr = "127.0.0.1:50000".parse().unwrap();
    let mut connector = build_client_connector(config, client_addr, &options).unwrap();

    let mut output = WriteBuf::new();
    assert!(matches!(
        connector.step_no_input(&mut output).unwrap(),
        Written::Size(_)
    ));

    let confirm = X224(nego::ConnectionConfirm::Response {
        flags: ResponseFlags::DYNVC_GFX_PROTOCOL_SUPPORTED,
        protocol: nego::SecurityProtocol::HYBRID,
    });
    let confirm_bytes = encode_vec(&confirm).unwrap();
    assert!(connector
        .step(&confirm_bytes, &mut output)
        .unwrap()
        .is_nothing());
    connector.mark_security_upgrade_as_done();
    connector.mark_credssp_as_done();

    let mut connect_initial = WriteBuf::new();
    assert!(matches!(
        connector.step_no_input(&mut connect_initial).unwrap(),
        Written::Size(_)
    ));

    let x224_payload = X224::<X224Data<'_>>::decode(&mut ReadCursor::new(connect_initial.filled()))
        .unwrap()
        .0;
    let decoded =
        mcs::ConnectInitial::decode(&mut ReadCursor::new(x224_payload.data.as_ref())).unwrap();
    let flags = decoded
        .conference_create_request
        .gcc_blocks()
        .core
        .optional_data
        .early_capability_flags
        .unwrap();

    assert!(flags.contains(ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL));
}

#[test]
fn h264_direct_egfx_advertises_modern_avc_capabilities_before_fallback() {
    let (mut egfx_client, _, _, _) = RdpEgfxBridge::new("rdp-egfx-capabilities".to_string());

    let messages = egfx_client.start(6).unwrap();

    assert_eq!(messages.len(), 1);
    let encoded = encode_vec(messages[0].as_ref()).unwrap();
    let mut cursor = ReadCursor::new(&encoded);
    let pdu: GfxPdu = decode_cursor(&mut cursor).unwrap();
    let GfxPdu::CapabilitiesAdvertise(caps) = pdu else {
        panic!("expected CapabilitiesAdvertise");
    };

    let advertised = caps
        .0
        .iter()
        .map(|cap| cap.parsed().unwrap().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(
        advertised,
        vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE,
            },
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::SMALL_CACHE | CapabilitiesV81Flags::AVC420_ENABLED,
            },
            CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            },
        ]
    );
}

#[test]
fn egfx_diagnostics_reports_v81_without_avc420_as_non_h264() {
    let diagnostics = egfx_capability_diagnostics(&CapabilitySet::V8_1 {
        flags: CapabilitiesV81Flags::SMALL_CACHE,
    });

    assert!(!diagnostics.avc420);
    assert!(!diagnostics.avc444);
    assert!(diagnostics.small_cache);
    assert!(!diagnostics.thin_client);
}

#[test]
fn egfx_non_avc_confirmation_reports_h264_direct_unavailable() {
    let (mut egfx_client, _encoded_rx, _bitmap_rx, mut status_rx) =
        RdpEgfxBridge::new("rdp-egfx-non-avc".to_string());
    let confirm =
        GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_typed(&CapabilitySet::V8_1 {
            flags: CapabilitiesV81Flags::SMALL_CACHE,
        }));
    let encoded = encode_vec(&confirm).unwrap();
    let mut compressor = Compressor::new();
    let payload =
        compress_and_wrap_egfx(&encoded, &mut compressor, CompressionMode::Never).unwrap();

    let messages = egfx_client.process(6, &payload).unwrap();

    assert!(messages.is_empty());
    let detail = status_rx.try_recv().unwrap();
    assert_eq!(
        detail,
        RdpStatusDetail::H264DirectUnavailable {
            reason: "Windows RDPGFX 未协商 AVC/H.264，服务端正在发送 ClearCodec/bitmap。"
                .to_string(),
        },
    );
    assert!(status_rx.try_recv().is_err());
}

#[test]
fn egfx_bitmap_update_is_forwarded_as_canvas_frame() {
    let (bitmap_tx, mut bitmap_rx) = mpsc::unbounded_channel();
    let mut bridge = RdpEgfxBridge::new_for_tests("rdp-egfx-bitmap".to_string(), bitmap_tx);

    bridge.on_reset_graphics(1280, 720);
    let mut client = ironrdp_egfx::client::GraphicsPipelineClient::new(Box::new(bridge), None);
    client
        .handle_pdu_for_tests(GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 4,
            width: 640,
            height: 480,
            pixel_format: EgfxPixelFormat::XRgb,
        }))
        .unwrap();
    client
        .handle_pdu_for_tests(GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 4,
            output_origin_x: 100,
            output_origin_y: 50,
        }))
        .unwrap();
    client
        .handle_pdu_for_tests(GfxPdu::WireToSurface1(WireToSurface1Pdu {
            surface_id: 4,
            destination_rectangle: ExclusiveRectangle {
                left: 10,
                top: 20,
                right: 12,
                bottom: 21,
            },
            codec_id: Codec1Type::Uncompressed,
            pixel_format: EgfxPixelFormat::XRgb,
            bitmap_data: vec![3, 2, 1, 255, 6, 5, 4, 255],
        }))
        .unwrap();

    let frame = bitmap_rx.try_recv().unwrap();
    assert_eq!(frame.session_id, "rdp-egfx-bitmap");
    assert_eq!(frame.surface_id, 4);
    assert_eq!(frame.width, 1280);
    assert_eq!(frame.height, 720);
    assert_eq!(frame.x, 110);
    assert_eq!(frame.y, 70);
    assert_eq!(frame.region_width, 2);
    assert_eq!(frame.region_height, 1);
    assert_eq!(frame.rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
}

#[test]
fn egfx_diagnostics_reports_v107_without_avc_disabled_as_h264() {
    let diagnostics = egfx_capability_diagnostics(&CapabilitySet::V10_7 {
        flags: CapabilitiesV107Flags::SMALL_CACHE,
    });

    assert!(diagnostics.avc420);
    assert!(diagnostics.avc444);
    assert!(diagnostics.small_cache);
    assert!(!diagnostics.thin_client);
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
fn rdp_connector_attaches_egfx_dynamic_channel_for_h264_direct_mode() {
    let mut options = test_rdp_options(true, true);
    options.transport_mode = RdpTransportMode::H264Direct;
    let credentials = test_rdp_credentials();
    let config = build_ironrdp_config(&options, &credentials).unwrap();
    let client_addr = "127.0.0.1:50000".parse().unwrap();

    let connector = build_client_connector(config, client_addr, &options).unwrap();
    let channels = rdp_static_channel_names(&connector);

    assert_eq!(
        channels,
        vec![
            "cliprdr".to_string(),
            "drdynvc".to_string(),
            "rdpsnd".to_string(),
        ]
    );
}

#[test]
fn h264_direct_keeps_egfx_and_rdpevor_encoded_video_receivers() {
    let mut options = test_rdp_options(true, true);
    options.transport_mode = RdpTransportMode::H264Direct;
    let credentials = test_rdp_credentials();
    let config = build_ironrdp_config(&options, &credentials).unwrap();
    let client_addr = "127.0.0.1:50000".parse().unwrap();

    let count =
        h264_direct_encoded_video_receiver_count_for_tests(config, client_addr, &options).unwrap();

    assert_eq!(count, 2);
}

#[test]
fn rdp_connector_labels_drdynvc_diagnostics_for_h264_direct_mode() {
    let mut options = test_rdp_options(true, true);
    options.transport_mode = RdpTransportMode::H264Direct;
    options.session_id = "rdp-diagnostics-session".to_string();
    let credentials = test_rdp_credentials();
    let config = build_ironrdp_config(&options, &credentials).unwrap();
    let client_addr = "127.0.0.1:50000".parse().unwrap();

    let connector = build_client_connector(config, client_addr, &options).unwrap();
    let (_, channel) = connector
        .static_channels
        .get_by_channel_name(&DrdynvcClient::NAME)
        .unwrap();
    let drdynvc = channel
        .channel_processor_downcast_ref::<DrdynvcClient>()
        .unwrap();

    assert_eq!(drdynvc.diagnostics_label(), Some("rdp-diagnostics-session"));
}

#[test]
fn h264_direct_accepts_windows_video_display_dynamic_channels() {
    for (index, channel_name) in WINDOWS_H264_DIRECT_DVC_NAMES.iter().enumerate() {
        let channel_id = 32 + index as u32;
        let mut drdynvc = build_h264_direct_drdynvc_for_tests("rdp-diagnostics-session");
        assert_eq!(
            dynamic_channel_create_status(&mut drdynvc, channel_id, channel_name),
            CreationStatus::OK,
            "{channel_name} should be accepted"
        );
    }
}

#[test]
fn negotiation_response_flags_diagnostics_identifies_gfx_support() {
    let flags =
        ResponseFlags::EXTENDED_CLIENT_DATA_SUPPORTED | ResponseFlags::DYNVC_GFX_PROTOCOL_SUPPORTED;

    let diagnostics = negotiation_response_flags_diagnostics(flags);

    assert!(diagnostics.extended_client_data);
    assert!(diagnostics.dynvc_gfx);
    assert_eq!(diagnostics.bits, flags.bits());
}

#[test]
fn pointer_button_mapping_uses_web_button_order() {
    assert_eq!(mouse_button_from_web(0).unwrap(), RdpMouseButton::Left);
    assert_eq!(mouse_button_from_web(1).unwrap(), RdpMouseButton::Middle);
    assert_eq!(mouse_button_from_web(2).unwrap(), RdpMouseButton::Right);
    assert!(mouse_button_from_web(9).is_err());
}

fn dynamic_channel_create_status(
    drdynvc: &mut DrdynvcClient,
    channel_id: u32,
    channel_name: &str,
) -> CreationStatus {
    let request =
        DrdynvcServerPdu::Create(CreateRequestPdu::new(channel_id, channel_name.to_string()));
    let payload = encode_vec(&request).unwrap();
    let responses = drdynvc.process(&payload).unwrap();

    responses
        .iter()
        .filter_map(|response| {
            let bytes = response.encode_unframed_pdu().unwrap();
            match DrdynvcClientPdu::decode(&mut ReadCursor::new(&bytes)).unwrap() {
                DrdynvcClientPdu::Create(create) => Some(create.creation_status()),
                _ => None,
            }
        })
        .next()
        .unwrap()
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
        transport_mode: RdpTransportMode::LegacyBitmap,
    }
}

fn test_rdp_credentials() -> RdpCredentials {
    RdpCredentials {
        username: "Administrator".to_string(),
        password: "secret".to_string(),
    }
}
