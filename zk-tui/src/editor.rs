//! Terminal-safe editor launching helpers shared by TUI screens.

use std::{ffi::OsStr, io::stdout, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

/// Describes whether an editor launch used a true diff command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorLaunchMode {
    /// Both paths were opened through an editor diff mode.
    Diff,
    /// Only the local path was opened.
    LocalOnly,
}

/// Opens one path in the configured editor.
pub(crate) fn open_path(editor: &OsStr, path: &Path) -> Result<()> {
    run_editor_command(CommandSpec {
        program: editor,
        args: vec![path.as_os_str()],
    })
}

/// Opens a local and remote path using the editor's built-in diff mode when known.
pub(crate) fn open_diff(
    editor: &OsStr,
    local: &Path,
    remote: Option<&Path>,
) -> Result<EditorLaunchMode> {
    let Some(remote) = remote else {
        open_path(editor, local)?;
        return Ok(EditorLaunchMode::LocalOnly);
    };

    match editor_name(editor).as_deref() {
        Some("code" | "code-insiders" | "codium") => {
            run_editor_command(CommandSpec {
                program: editor,
                args: vec![OsStr::new("--diff"), local.as_os_str(), remote.as_os_str()],
            })?;
            Ok(EditorLaunchMode::Diff)
        }
        Some("vim" | "vi" | "vimdiff") => {
            run_editor_command(CommandSpec {
                program: OsStr::new("vimdiff"),
                args: vec![local.as_os_str(), remote.as_os_str()],
            })?;
            Ok(EditorLaunchMode::Diff)
        }
        Some("nvim") => {
            run_editor_command(CommandSpec {
                program: OsStr::new("nvim"),
                args: vec![OsStr::new("-d"), local.as_os_str(), remote.as_os_str()],
            })?;
            Ok(EditorLaunchMode::Diff)
        }
        _ => {
            open_path(editor, local)?;
            Ok(EditorLaunchMode::LocalOnly)
        }
    }
}

struct CommandSpec<'a> {
    program: &'a OsStr,
    args: Vec<&'a OsStr>,
}

fn run_editor_command(spec: CommandSpec<'_>) -> Result<()> {
    suspend_terminal().context("while attempting to suspend TUI for editor")?;
    let status_result = Command::new(spec.program)
        .args(spec.args)
        .status()
        .context("while attempting to launch editor");
    resume_terminal().context("while attempting to restore TUI after editor")?;
    let status = status_result?;

    if !status.success() {
        bail!("editor exited with status {status}");
    }
    Ok(())
}

fn editor_name(editor: &OsStr) -> Option<String> {
    Path::new(editor)
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
}

fn suspend_terminal() -> Result<()> {
    disable_raw_mode().context("while attempting to disable terminal raw mode")?;
    execute!(stdout(), LeaveAlternateScreen).context("while attempting to leave alternate screen")
}

fn resume_terminal() -> Result<()> {
    execute!(stdout(), EnterAlternateScreen)
        .context("while attempting to enter alternate screen")?;
    enable_raw_mode().context("while attempting to enable terminal raw mode")
}
