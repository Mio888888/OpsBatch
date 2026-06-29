fn main() {
    // 在 Windows 下注入自定义应用清单，声明 PerMonitorV2 DPI 感知，
    // 修复高分屏(>100% 缩放)下 WebView2 窗口字体/画面模糊的问题。
    #[cfg(windows)]
    let attributes = tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new().app_manifest(include_str!("opsbatch.exe.manifest")),
    );
    #[cfg(not(windows))]
    let attributes = tauri_build::Attributes::new();

    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
