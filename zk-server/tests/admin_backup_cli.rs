// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use zk_backup::manifest::{BackupManifest, list_manifests};

use common::{unique_test_dir, write_text};

struct BackupCliHarness {
    root: PathBuf,
    output_dir: PathBuf,
    config_path: PathBuf,
}

impl BackupCliHarness {
    fn new(name: &str, backup_enabled: bool) -> Self {
        let root = unique_test_dir(name);
        let vault_path = root.join("vault");
        let output_dir = root.join("backups");
        let database_path = root.join("db").join("server.sqlite");
        let config_path = root.join("server.toml");
        fs::create_dir_all(
            database_path
                .parent()
                .expect("database path should have parent"),
        )
        .expect("database parent should be created");
        write_text(&vault_path.join("note.md"), "# Note\n");
        write_text(&vault_path.join("asset.txt"), "asset");
        write_config(
            &config_path,
            &vault_path,
            &database_path,
            backup_enabled.then_some(output_dir.as_path()),
        );

        Self {
            root,
            output_dir,
            config_path,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("zk-server").expect("zk-server binary should build");

        command
            .env("HOME", &self.root)
            .env("APPDATA", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .env_remove("ZK_SERVER_CONFIG")
            .env_remove("ZK_SERVER_VAULT_PATH")
            .env_remove("ZK_SERVER_DATABASE_PATH")
            .env_remove("ZK_SERVER_LISTEN_ADDR")
            .env_remove("ZK_SERVER_LOG_FILTER")
            .env_remove("ZK_SERVER_TOKEN_SHA256")
            .env_remove("ZK_SERVER_TRASH_RETENTION_DAYS")
            .env_remove("ZK_SERVER_MAX_BATCH_SIZE")
            .env_remove("ZK_SERVER_BACKUP_OUTPUT_DIR")
            .env_remove("ZK_SERVER_BACKUP_RETENTION_COUNT")
            .env_remove("ZK_SERVER_BACKUP_SCHEDULE_CRON")
            .env_remove("ZK_SERVER_BACKUP_AGE_RECIPIENT")
            .env_remove("ZK_SERVER_BACKUP_INCLUDE_ATTACHMENTS")
            .env_remove("ZK_SERVER_BACKUP_INCLUDE_SERVER_DB");

        command
    }
}

#[test]
fn list_empty_output_dir_prints_no_backups_found() {
    let fixture = BackupCliHarness::new("admin-backup-cli-empty-list", true);

    let assert = fixture
        .command()
        .arg("admin")
        .arg("backup")
        .arg("list")
        .arg("--config")
        .arg(&fixture.config_path)
        .assert()
        .success();
    let stdout = stdout_text(assert.get_output());

    assert_eq!(stdout, "no backups found\n");
}

#[test]
fn run_writes_manifest_and_list_show_reflect_it() {
    let fixture = BackupCliHarness::new("admin-backup-cli-run", true);

    let run_assert = fixture
        .command()
        .arg("admin")
        .arg("backup")
        .arg("run")
        .arg("--config")
        .arg(&fixture.config_path)
        .assert()
        .success();
    let run_stdout = stdout_text(run_assert.get_output());
    let backup_id = field_value(&run_stdout, "backup_id");

    assert!(run_stdout.contains("archive: backup-"));
    assert!(run_stdout.contains("sha256: "));
    assert!(run_stdout.contains("size_bytes: "));
    assert_eq!(manifests(&fixture.output_dir).len(), 1);

    let list_assert = fixture
        .command()
        .arg("admin")
        .arg("backup")
        .arg("list")
        .arg("--config")
        .arg(&fixture.config_path)
        .assert()
        .success();
    let list_stdout = stdout_text(list_assert.get_output());
    assert!(list_stdout.contains("BACKUP_ID"));
    assert!(list_stdout.contains(backup_id));

    let show_assert = fixture
        .command()
        .arg("admin")
        .arg("backup")
        .arg("show")
        .arg(backup_id)
        .arg("--config")
        .arg(&fixture.config_path)
        .assert()
        .success();
    let show_stdout = stdout_text(show_assert.get_output());
    assert!(show_stdout.contains("backup_id: "));
    assert!(show_stdout.contains(backup_id));
    assert!(show_stdout.contains("source_vault_path: "));
}

#[test]
fn json_output_round_trips_through_manifest_types() {
    let fixture = BackupCliHarness::new("admin-backup-cli-json", true);
    fixture
        .command()
        .arg("admin")
        .arg("backup")
        .arg("run")
        .arg("--config")
        .arg(&fixture.config_path)
        .assert()
        .success();
    let manifest = manifests(&fixture.output_dir)
        .into_iter()
        .next()
        .expect("run should create one manifest");

    let list_assert = fixture
        .command()
        .arg("admin")
        .arg("backup")
        .arg("list")
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--json")
        .assert()
        .success();
    let listed: Vec<BackupManifest> = serde_json::from_slice(&list_assert.get_output().stdout)
        .expect("list JSON should deserialize");
    assert_eq!(listed, vec![manifest.clone()]);

    let show_assert = fixture
        .command()
        .arg("admin")
        .arg("backup")
        .arg("show")
        .arg(&manifest.backup_id)
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--json")
        .assert()
        .success();
    let shown: BackupManifest = serde_json::from_slice(&show_assert.get_output().stdout)
        .expect("show JSON should deserialize");
    assert_eq!(shown, manifest);
}

#[test]
fn missing_backup_config_returns_clear_error() {
    let fixture = BackupCliHarness::new("admin-backup-cli-missing-config", false);

    let assert = fixture
        .command()
        .arg("admin")
        .arg("backup")
        .arg("run")
        .arg("--config")
        .arg(&fixture.config_path)
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("backup config is disabled"));
}

#[test]
fn config_env_wins_over_config_argument() {
    let env_fixture = BackupCliHarness::new("admin-backup-cli-env-config", false);
    let arg_fixture = BackupCliHarness::new("admin-backup-cli-arg-config", true);
    let mut command = arg_fixture.command();

    let assert = command
        .env("ZK_SERVER_CONFIG", &env_fixture.config_path)
        .arg("admin")
        .arg("backup")
        .arg("list")
        .arg("--config")
        .arg(&arg_fixture.config_path)
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("backup config is disabled"));
}

fn write_config(
    config_path: &Path,
    vault_path: &Path,
    database_path: &Path,
    output_dir: Option<&Path>,
) {
    let listen_addr: SocketAddr = "127.0.0.1:7878"
        .parse()
        .expect("test listen addr should parse");
    let backup_section = output_dir.map_or_else(String::new, |path| {
        format!(
            r#"
[backup]
output_dir = "{output_dir}"
retention_count = 10
include_attachments = true
include_server_db = false
"#,
            output_dir = path.display()
        )
    });

    write_text(
        config_path,
        &format!(
            r#"
vault_path = "{vault_path}"
database_path = "{database_path}"
listen_addr = "{listen_addr}"
{backup_section}
"#,
            vault_path = vault_path.display(),
            database_path = database_path.display(),
        ),
    );
}

fn manifests(output_dir: &Path) -> Vec<BackupManifest> {
    list_manifests(output_dir).expect("backup manifest directory should list")
}

fn stdout_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn field_value<'a>(text: &'a str, field_name: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{field_name}: ")))
        .unwrap_or_else(|| panic!("{field_name} field should be present"))
}
