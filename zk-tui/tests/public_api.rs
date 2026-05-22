// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use std::any::{type_name, type_name_of_val};

use zk_tui::{
    app::{
        App, ConfirmDialog, FeatureFlags, Modal, Screen, ScreenAction, ScreenCtx, ScreenId,
        StatusMessages,
    },
    event::{
        AppEvent, AppEventSender, BackgroundResult, Clock, EventLoop, SystemClock,
        TerminalEventSource,
    },
    jobs::{
        ConflictItemKind, ConflictListItem, ConflictListResult, ConflictResolutionOutcome,
        ConflictResolutionStrategy, ConflictResolutionTarget, IssueDeviceTokenIntent,
        IssuedDeviceTokenOutcome, JobContext, JobHandle, JobKind, JobResult, JobRunner,
        JobRunnerHandle, JobSpec, MockJob, MockJobResult, NoteBodyPreview, NoteBodyResult,
        NoteFilter, NoteListRequest, NoteListResult, ResolveConflictIntent, ScanArgs, SyncIntent,
        SyncOutcome, TrashIntent, TrashOutcome,
    },
    keymap::{KeyChord, global_keybindings},
    screens::{
        admin::AdminScreen, auth::AuthScreen, doctor::DoctorScreen, help::HelpScreen,
        notes::NotesScreen, overview::OverviewScreen, resolve_conflict::ResolveConflictScreen,
        sync::SyncScreen, trash::TrashScreen,
    },
    terminal::{TerminalGuard, install_panic_hook},
    theme::{Theme, ThemeKind, ThemeParseError},
    widgets::{
        status_bar::StatusBar,
        toast::{Toast, ToastLine, ToastQueue, ToastSeverity},
    },
};

#[test]
fn tui_public_api_is_available_from_public_modules() {
    assert!(!type_name::<App>().is_empty());
    assert!(!type_name::<ConfirmDialog>().is_empty());
    assert!(!type_name::<FeatureFlags>().is_empty());
    assert!(!type_name::<Modal>().is_empty());
    assert!(!type_name::<dyn Screen>().is_empty());
    assert!(!type_name::<ScreenAction>().is_empty());
    assert!(!type_name::<ScreenCtx>().is_empty());
    assert!(!type_name::<ScreenId>().is_empty());
    assert!(!type_name::<StatusMessages>().is_empty());

    assert!(!type_name::<AppEvent>().is_empty());
    assert!(!type_name::<AppEventSender>().is_empty());
    assert!(!type_name::<BackgroundResult>().is_empty());
    assert!(!type_name::<dyn Clock>().is_empty());
    assert!(!type_name::<EventLoop>().is_empty());
    assert!(!type_name::<SystemClock>().is_empty());
    assert!(!type_name::<dyn TerminalEventSource>().is_empty());

    assert!(!type_name::<JobContext>().is_empty());
    assert!(!type_name::<JobHandle>().is_empty());
    assert!(!type_name::<JobKind>().is_empty());
    assert!(!type_name::<JobResult>().is_empty());
    assert!(!type_name::<JobRunner>().is_empty());
    assert!(!type_name::<JobRunnerHandle>().is_empty());
    assert!(!type_name::<JobSpec>().is_empty());
    assert!(!type_name::<MockJob>().is_empty());
    assert!(!type_name::<MockJobResult>().is_empty());
    assert!(!type_name::<ScanArgs>().is_empty());
    assert!(!type_name::<NoteFilter>().is_empty());
    assert!(!type_name::<NoteListRequest>().is_empty());
    assert!(!type_name::<NoteListResult>().is_empty());
    assert!(!type_name::<NoteBodyPreview>().is_empty());
    assert!(!type_name::<NoteBodyResult>().is_empty());
    assert!(!type_name::<ConflictItemKind>().is_empty());
    assert!(!type_name::<ConflictListItem>().is_empty());
    assert!(!type_name::<ConflictListResult>().is_empty());
    assert!(!type_name::<ConflictResolutionStrategy>().is_empty());
    assert!(!type_name::<ConflictResolutionTarget>().is_empty());
    assert!(!type_name::<ResolveConflictIntent>().is_empty());
    assert!(!type_name::<ConflictResolutionOutcome>().is_empty());
    assert!(!type_name::<IssueDeviceTokenIntent>().is_empty());
    assert!(!type_name::<IssuedDeviceTokenOutcome>().is_empty());
    assert!(!type_name::<SyncIntent>().is_empty());
    assert!(!type_name::<SyncOutcome>().is_empty());
    assert!(!type_name::<TrashIntent>().is_empty());
    assert!(!type_name::<TrashOutcome>().is_empty());

    assert!(!type_name::<KeyChord>().is_empty());
    assert!(!type_name::<AdminScreen>().is_empty());
    assert!(!type_name::<AuthScreen>().is_empty());
    assert!(!type_name::<DoctorScreen>().is_empty());
    assert!(!type_name::<HelpScreen>().is_empty());
    assert!(!type_name::<NotesScreen>().is_empty());
    assert!(!type_name::<OverviewScreen>().is_empty());
    assert!(!type_name::<ResolveConflictScreen>().is_empty());
    assert!(!type_name::<SyncScreen>().is_empty());
    assert!(!type_name::<TrashScreen>().is_empty());

    assert!(!type_name::<TerminalGuard>().is_empty());
    assert!(!type_name::<Theme>().is_empty());
    assert!(!type_name::<ThemeKind>().is_empty());
    assert!(!type_name::<ThemeParseError>().is_empty());
    assert!(!type_name::<StatusBar<'static>>().is_empty());
    assert!(!type_name::<Toast>().is_empty());
    assert!(!type_name::<ToastLine<'static>>().is_empty());
    assert!(!type_name::<ToastQueue>().is_empty());
    assert!(!type_name::<ToastSeverity>().is_empty());

    assert!(!type_name_of_val(&global_keybindings).is_empty());
    assert!(!type_name_of_val(&install_panic_hook).is_empty());
}

#[cfg(feature = "ai")]
#[test]
fn tui_ai_public_api_is_available_from_public_modules() {
    use zk_tui::{
        jobs::{
            ChatConversationCreatedResult, ChatConversationDetailResult,
            ChatConversationListResult, ChatConversationMutationResult, ChatStreamCitation,
            ChatStreamDelta, ChatStreamDone, SemanticSearchResult, SemanticSearchResultItem,
            WikiCancelResult, WikiIngestRequest, WikiIngestResult, WikiJobsResult, WikiLintRequest,
            WikiLintResult,
        },
        screens::{chat::ChatScreen, search::SearchScreen, wiki::WikiScreen},
    };

    assert!(!type_name::<SemanticSearchResult>().is_empty());
    assert!(!type_name::<SemanticSearchResultItem>().is_empty());
    assert!(!type_name::<WikiIngestRequest>().is_empty());
    assert!(!type_name::<WikiIngestResult>().is_empty());
    assert!(!type_name::<WikiLintRequest>().is_empty());
    assert!(!type_name::<WikiLintResult>().is_empty());
    assert!(!type_name::<WikiJobsResult>().is_empty());
    assert!(!type_name::<WikiCancelResult>().is_empty());
    assert!(!type_name::<ChatConversationListResult>().is_empty());
    assert!(!type_name::<ChatConversationDetailResult>().is_empty());
    assert!(!type_name::<ChatConversationCreatedResult>().is_empty());
    assert!(!type_name::<ChatConversationMutationResult>().is_empty());
    assert!(!type_name::<ChatStreamDelta>().is_empty());
    assert!(!type_name::<ChatStreamCitation>().is_empty());
    assert!(!type_name::<ChatStreamDone>().is_empty());
    assert!(!type_name::<ChatScreen>().is_empty());
    assert!(!type_name::<SearchScreen>().is_empty());
    assert!(!type_name::<WikiScreen>().is_empty());
}
