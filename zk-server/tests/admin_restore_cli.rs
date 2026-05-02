// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

mod common;

use std::{
    collections::BTreeMap,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use assert_cmd::Command;
use secrecy::ExposeSecret;
use zk_backup::manifest::{BackupManifest, list_manifests};

use common::{unique_test_dir, write_text};

struct RestoreCliHarness {
    root: PathBuf,
    vault_path: PathBuf,
    output_dir: PathBuf,
    config_path: PathBuf,
}

impl RestoreCliHarness {
    fn new(name: &str, age_recipient: Option<&str>) -> Self {
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
        write_text(&vault_path.join("nested/second.md"), "# Second\n");
        write_text(&vault_path.join("assets/image.txt"), "asset");
        write_config(
            &config_path,
            &vault_path,
            &database_path,
            &output_dir,
            age_recipient,
        );

        Self {
            root,
            vault_path,
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

    fn run_backup(&self) -> BackupManifest {
        let assert = self
            .command()
            .arg("admin")
            .arg("backup")
            .arg("run")
            .arg("--config")
            .arg(&self.config_path)
            .assert()
            .success();
        let stdout = stdout_text(assert.get_output());
        let backup_id = field_value(&stdout, "backup_id");

        manifests(&self.output_dir)
            .into_iter()
            .find(|manifest| manifest.backup_id == backup_id)
            .expect("created backup manifest should be present")
    }
}

#[test]
fn plain_backup_restores_markdown_files() {
    let fixture = RestoreCliHarness::new("admin-restore-cli-plain", None);
    let manifest = fixture.run_backup();
    let target_dir = fixture.root.join("restore");

    let assert = fixture
        .command()
        .arg("admin")
        .arg("restore")
        .arg(backup_prefix(&manifest.backup_id))
        .arg("--target")
        .arg(&target_dir)
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--yes")
        .assert()
        .success();
    let stdout = stdout_text(assert.get_output());

    assert_eq!(
        markdown_files_by_relative_path(&target_dir),
        markdown_files_by_relative_path(&fixture.vault_path)
    );
    assert!(stdout.contains("note_count: 2"));
    assert!(stdout.contains("asset_count: 1"));
    assert!(stdout.contains("bytes_extracted: "));
}

#[test]
fn age_backup_restores_with_string_identity() {
    let identity = age::x25519::Identity::generate();
    let identity_string = identity.to_string();
    let identity_arg = format!("string:{}", identity_string.expose_secret());
    let fixture = RestoreCliHarness::new(
        "admin-restore-cli-age",
        Some(identity.to_public().to_string().as_str()),
    );
    let manifest = fixture.run_backup();
    let target_dir = fixture.root.join("restore-age");

    fixture
        .command()
        .arg("admin")
        .arg("restore")
        .arg(backup_prefix(&manifest.backup_id))
        .arg("--target")
        .arg(&target_dir)
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--age-identity")
        .arg(identity_arg)
        .arg("--yes")
        .assert()
        .success();

    assert_eq!(
        markdown_files_by_relative_path(&target_dir),
        markdown_files_by_relative_path(&fixture.vault_path)
    );
}

#[test]
fn ambiguous_backup_prefix_returns_candidates() {
    let fixture = RestoreCliHarness::new("admin-restore-cli-ambiguous", None);
    let first = fixture.run_backup();
    let second = fixture.run_backup();
    let prefix = common_prefix(&first.backup_id, &second.backup_id);
    let target_dir = fixture.root.join("restore-ambiguous");

    let assert = fixture
        .command()
        .arg("admin")
        .arg("restore")
        .arg(prefix)
        .arg("--target")
        .arg(&target_dir)
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--yes")
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("backup id prefix is ambiguous"));
    assert!(stderr.contains(&first.backup_id));
    assert!(stderr.contains(&second.backup_id));
}

#[test]
fn dry_run_prints_summary_without_restoring_files() {
    let fixture = RestoreCliHarness::new("admin-restore-cli-dry-run", None);
    let manifest = fixture.run_backup();
    let target_dir = fixture.root.join("restore-dry-run");

    let assert = fixture
        .command()
        .arg("admin")
        .arg("restore")
        .arg(backup_prefix(&manifest.backup_id))
        .arg("--target")
        .arg(&target_dir)
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--dry-run")
        .assert()
        .success();
    let stdout = stdout_text(assert.get_output());

    assert!(stdout.contains("note_count: 2"));
    assert!(stdout.contains("asset_count: 1"));
    assert!(target_dir.is_dir());
    assert!(
        fs::read_dir(target_dir)
            .expect("dry-run target should be readable")
            .next()
            .is_none()
    );
}

#[test]
fn non_empty_target_without_overwrite_returns_clear_error() {
    let fixture = RestoreCliHarness::new("admin-restore-cli-non-empty", None);
    let manifest = fixture.run_backup();
    let target_dir = fixture.root.join("restore-non-empty");
    write_text(&target_dir.join("existing.txt"), "existing");

    let assert = fixture
        .command()
        .arg("admin")
        .arg("restore")
        .arg(backup_prefix(&manifest.backup_id))
        .arg("--target")
        .arg(&target_dir)
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--yes")
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("target_dir not empty"));
}

#[test]
fn encrypted_backup_without_age_identity_returns_clear_error() {
    let identity = age::x25519::Identity::generate();
    let fixture = RestoreCliHarness::new(
        "admin-restore-cli-age-missing-identity",
        Some(identity.to_public().to_string().as_str()),
    );
    let manifest = fixture.run_backup();
    let target_dir = fixture.root.join("restore-missing-identity");

    let assert = fixture
        .command()
        .arg("admin")
        .arg("restore")
        .arg(backup_prefix(&manifest.backup_id))
        .arg("--target")
        .arg(&target_dir)
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--yes")
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("age identity is required"));
}

#[test]
fn plain_backup_with_age_identity_returns_clear_error() {
    let fixture = RestoreCliHarness::new("admin-restore-cli-plain-with-identity", None);
    let manifest = fixture.run_backup();
    let target_dir = fixture.root.join("restore-plain-with-identity");

    let assert = fixture
        .command()
        .arg("admin")
        .arg("restore")
        .arg(backup_prefix(&manifest.backup_id))
        .arg("--target")
        .arg(&target_dir)
        .arg("--config")
        .arg(&fixture.config_path)
        .arg("--age-identity")
        .arg("string:not-used")
        .arg("--yes")
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("age identity is only valid for encrypted backups"));
}

#[test]
fn wrong_confirmation_rejects_restore_without_writing_files() {
    let fixture = RestoreCliHarness::new("admin-restore-cli-confirmation", None);
    let manifest = fixture.run_backup();
    let target_dir = fixture.root.join("restore-confirmation");

    let assert = fixture
        .command()
        .arg("admin")
        .arg("restore")
        .arg(backup_prefix(&manifest.backup_id))
        .arg("--target")
        .arg(&target_dir)
        .arg("--config")
        .arg(&fixture.config_path)
        .write_stdin("cancel\n")
        .assert()
        .failure();
    let stderr = stderr_text(assert.get_output());

    assert!(stderr.contains("restore confirmation did not match"));
    assert!(!target_dir.exists());
}

fn write_config(
    config_path: &Path,
    vault_path: &Path,
    database_path: &Path,
    output_dir: &Path,
    age_recipient: Option<&str>,
) {
    let listen_addr: SocketAddr = "127.0.0.1:7878"
        .parse()
        .expect("test listen addr should parse");
    let encryption_section = age_recipient.map_or_else(String::new, |recipient| {
        format!(
            r#"
[backup.encryption]
type = "age"
recipient = "{recipient}"
"#
        )
    });

    write_text(
        config_path,
        &format!(
            r#"
vault_path = "{vault_path}"
database_path = "{database_path}"
listen_addr = "{listen_addr}"

[backup]
output_dir = "{output_dir}"
retention_count = 10
include_attachments = true
include_server_db = false
{encryption_section}
"#,
            vault_path = vault_path.display(),
            database_path = database_path.display(),
            output_dir = output_dir.display(),
        ),
    );
}

fn manifests(output_dir: &Path) -> Vec<BackupManifest> {
    list_manifests(output_dir).expect("backup manifest directory should list")
}

fn markdown_files_by_relative_path(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect_markdown_files(root, root, &mut files);

    files
}

fn collect_markdown_files(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry_result in fs::read_dir(dir).expect("directory should be readable") {
        let entry = entry_result.expect("directory entry should be readable");
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(root, &path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            let relative_path = path
                .strip_prefix(root)
                .expect("file should be below root")
                .to_path_buf();
            files.insert(
                relative_path,
                fs::read(path).expect("markdown file should be readable"),
            );
        }
    }
}

fn backup_prefix(backup_id: &str) -> &str {
    &backup_id[..8]
}

fn common_prefix<'a>(left: &'a str, right: &str) -> &'a str {
    let prefix_len = left
        .char_indices()
        .zip(right.chars())
        .take_while(|((_, left_char), right_char)| left_char == right_char)
        .map(|((index, left_char), _)| index + left_char.len_utf8())
        .last()
        .expect("backup ids should share a ULID timestamp prefix");

    &left[..prefix_len]
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
