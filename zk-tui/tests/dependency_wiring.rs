use ratatui::layout::Rect;
use tokio::runtime::Builder;
use zk_tui as _;

#[test]
fn runtime_dependency_surface_is_wired() {
    assert!(std::mem::size_of::<crossterm::event::Event>() > 0);
    let _rect = Rect::default();
    let _error = anyhow::anyhow!("dependency smoke test");
    let level = tracing::Level::INFO;
    let _filter = tracing_subscriber::EnvFilter::try_new("info").expect("env filter should parse");
    let _runtime = Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("current-thread runtime should build");
    let _json = serde_json::json!({ "theme": "default" });
    let _toml: toml::Value = toml::from_str("theme = 'default'").expect("TOML should parse");
    let _url = url::Url::parse("https://zk.example.com").expect("URL should parse");
    let _overrides = zk_cli_core::CliOverrides::default();
    let _effective = zk_app::EffectiveConfig::default();
    let item_type = zk_domain::ItemType::Note;
    let _transport = zk_sync::NoopTransport::new();

    assert_eq!(level.as_str(), "INFO");
    assert!(matches!(item_type, zk_domain::ItemType::Note));
}

#[test]
fn serde_derive_is_available_for_tui_config() {
    #[derive(Debug, serde::Deserialize, serde::Serialize)]
    struct ThemeConfig {
        name: String,
    }

    let config = ThemeConfig {
        name: "default".to_string(),
    };

    let encoded = serde_json::to_string(&config).expect("theme config should serialize");
    let decoded: ThemeConfig =
        serde_json::from_str(&encoded).expect("theme config should deserialize");

    assert_eq!(decoded.name, "default");
}

#[cfg(feature = "ai")]
#[test]
fn ai_dependency_surface_is_wired() {
    let _provider = zk_ai::StubEmbeddingsProvider::new(8);
    let _connection = zk_index::open_memory().expect("in-memory index should open");
    let _vault_root: &zk_vault::VaultRoot = std::path::Path::new(".");
    let _model = "stub/sha256-8"
        .parse::<zk_domain::EmbeddingModelId>()
        .expect("embedding model id should parse");
}
