#![allow(unused_crate_dependencies)]
// This integration test keeps the initial public TUI API reachable.

use std::io::Stdout;

#[test]
fn terminal_lifecycle_api_is_public() {
    type DefaultTerminal = ratatui::Terminal<ratatui::backend::CrosstermBackend<Stdout>>;

    fn assert_init_signature(
        _init: fn() -> anyhow::Result<(zk_tui::terminal::TerminalGuard, DefaultTerminal)>,
    ) {
    }

    fn assert_hook_signature(_install_hook: fn()) {}

    assert_init_signature(zk_tui::terminal::TerminalGuard::init);
    assert_hook_signature(zk_tui::terminal::install_panic_hook);
}
