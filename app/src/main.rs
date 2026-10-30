fn main() {
    // The MCP stdio launcher is this executable without the UI: no window, no database, only stdin / stdout.
    if std::env::args().any(|a| a == "--mcp-stdio") {
        std::process::exit(opsbatch_app::mcp_bridge::run_stdio_launcher());
    }
    opsbatch_app::logging::BridgeLogger::init("warn");
    ops_core::tls::install_default_crypto_provider();

    let data_dir = ops_core::paths::run_data_dir();
    let core = match ops_core::Core::open(&data_dir) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("failed to open data directory {}: {error}", data_dir.display());
            std::process::exit(1);
        }
    };
    opsbatch_app::logging::attach(&core);
    ops_core::log::append_diagnostic_log(&data_dir, "startup", &format!("app started data_dir={}", data_dir.display()));
    ops_core::log::emit_log(&core, "info", "system", &ops_i18n::t!("app.title"), "backend");
    let ctx = ops_ssh::Ctx::new(core);
    opsbatch_app::app::run(ctx);
}
