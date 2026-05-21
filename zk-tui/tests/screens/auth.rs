use crossterm::event::KeyCode;
use zeroize::Zeroizing;
use zk_domain::AuthScope;
use zk_tui::{
    app::{Screen, ScreenAction},
    jobs::{IssueDeviceTokenIntent, IssuedDeviceTokenOutcome, JobKind, JobResult, JobSpec},
    screens::auth::AuthScreen,
};

use super::helpers::{
    assert_screen_snapshot, background, configured_context, job_error_event, key, render_screen,
    type_chars,
};

const PLAINTEXT: &str = "zk_live_plaintext_token";

#[test]
fn auth_empty_yaml_snapshot() {
    let ctx = super::helpers::test_context();
    let mut screen = AuthScreen::default();

    assert_screen_snapshot("screens__auth__empty", &render_screen(&mut screen, &ctx));
}

#[test]
fn auth_in_flight_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AuthScreen::default();
    start_issue(&mut screen, &mut ctx);

    assert_screen_snapshot(
        "screens__auth__in_flight",
        &render_screen(&mut screen, &ctx),
    );
}

#[test]
fn auth_success_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AuthScreen::default();
    start_issue(&mut screen, &mut ctx);
    assert!(matches!(
        screen.handle_event(&issued_event(), &mut ctx),
        ScreenAction::Stay
    ));

    let rows = render_screen(&mut screen, &ctx)
        .into_iter()
        .map(|row| row.replace(PLAINTEXT, "<redacted-token>"))
        .collect::<Vec<_>>();
    assert_screen_snapshot("screens__auth__success", &rows);
}

#[test]
fn auth_error_yaml_snapshot() {
    let mut ctx = configured_context();
    let mut screen = AuthScreen::default();
    start_issue(&mut screen, &mut ctx);
    assert!(matches!(
        screen.handle_event(
            &job_error_event(JobKind::IssueDeviceToken, "unauthorized"),
            &mut ctx
        ),
        ScreenAction::Stay
    ));

    assert_screen_snapshot("screens__auth__error", &render_screen(&mut screen, &ctx));
}

fn start_issue(screen: &mut AuthScreen, ctx: &mut zk_tui::app::ScreenCtx) {
    type_chars(screen, ctx, "admin");
    let _ = screen.handle_event(&key(KeyCode::Down), ctx);
    type_chars(screen, ctx, "laptop");
    assert!(matches!(
        screen.handle_event(&key(KeyCode::Enter), ctx),
        ScreenAction::Stay
    ));
    type_chars(screen, ctx, "secret");
    assert_issue_action(&screen.handle_event(&key(KeyCode::Enter), ctx));
}

fn issued_event() -> zk_tui::event::AppEvent {
    background(JobResult::IssuedDeviceToken(IssuedDeviceTokenOutcome {
        label: "laptop".to_string(),
        scopes: vec![AuthScope::Sync],
        token_prefix: "zk_liv".to_string(),
        plaintext: Zeroizing::new(PLAINTEXT.to_string()),
        created_at: "2026-05-15T12:00:00Z".to_string(),
    }))
}

fn assert_issue_action(action: &ScreenAction) {
    let ScreenAction::EnqueueJob(JobSpec::IssueDeviceToken(IssueDeviceTokenIntent {
        username,
        label,
        scopes,
        password,
    })) = action
    else {
        panic!("expected issue device token job");
    };

    assert_eq!(username, "admin");
    assert_eq!(label, "laptop");
    assert_eq!(scopes, &[AuthScope::Sync]);
    assert_eq!(password.as_str(), "secret");
}
