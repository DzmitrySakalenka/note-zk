//! Backup archive encryption primitives.

use std::{
    fs,
    io::{self, BufReader, BufWriter},
    path::{Path, PathBuf},
};

use data_encoding::HEXLOWER;
use sha2::{Digest, Sha256};

use crate::{BackupError, BackupResult};

/// Encrypts a backup archive from one filesystem path to another.
pub trait BackupEncryptor: Send + Sync {
    /// Returns the final archive extension for this encryption mode.
    fn extension(&self) -> &'static str;

    /// Encrypts or copies `plaintext_path` into `ciphertext_path`.
    ///
    /// # Errors
    ///
    /// Returns [`BackupError::Io`] for filesystem errors, or
    /// [`BackupError::Encryption`] for encryption failures.
    fn encrypt_in_place(&self, plaintext_path: &Path, ciphertext_path: &Path) -> BackupResult<()>;
}

/// Decrypts a backup archive from one filesystem path to another.
pub trait BackupDecryptor: Send + Sync {
    /// Decrypts `ciphertext_path` into `plaintext_path`.
    ///
    /// # Errors
    ///
    /// Returns [`BackupError::Io`] for filesystem errors, or
    /// [`BackupError::Encryption`] for decryption failures.
    fn decrypt_in_place(&self, ciphertext_path: &Path, plaintext_path: &Path) -> BackupResult<()>;
}

/// Encryptor used when backup encryption is disabled.
pub struct PassthroughEncryptor;

impl BackupEncryptor for PassthroughEncryptor {
    fn extension(&self) -> &'static str {
        ".zst"
    }

    fn encrypt_in_place(&self, plaintext_path: &Path, ciphertext_path: &Path) -> BackupResult<()> {
        write_atomic_file(ciphertext_path, |temp_file| {
            let mut input = BufReader::new(fs::File::open(plaintext_path)?);
            let mut output = BufWriter::new(temp_file);
            io::copy(&mut input, &mut output)?;
            finish_buffered_file(output)
        })
    }
}

/// Age recipient encryptor for backup archives.
pub struct AgeRecipientEncryptor {
    recipient: age::x25519::Recipient,
    fingerprint: String,
}

impl AgeRecipientEncryptor {
    /// Parses an age X25519 recipient string.
    ///
    /// # Errors
    ///
    /// Returns [`BackupError::Encryption`] when `recipient` is not a valid
    /// canonical age recipient string.
    pub fn from_recipient_string(recipient: &str) -> BackupResult<Self> {
        let parsed_recipient = recipient
            .parse::<age::x25519::Recipient>()
            .map_err(|error| encryption_error(&error))?;

        Ok(Self {
            recipient: parsed_recipient,
            fingerprint: recipient_fingerprint(recipient),
        })
    }

    /// Returns the manifest-safe recipient fingerprint.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

impl BackupEncryptor for AgeRecipientEncryptor {
    fn extension(&self) -> &'static str {
        ".zst.age"
    }

    fn encrypt_in_place(&self, plaintext_path: &Path, ciphertext_path: &Path) -> BackupResult<()> {
        write_atomic_file(ciphertext_path, |temp_file| {
            let mut input = BufReader::new(fs::File::open(plaintext_path)?);
            let output = BufWriter::new(temp_file);
            let encryptor = age::Encryptor::with_recipients(std::iter::once(&self.recipient as _))
                .map_err(|error| encryption_error(&error))?;
            let mut encrypted_output = encryptor.wrap_output(output)?;
            io::copy(&mut input, &mut encrypted_output)?;
            let output = encrypted_output.finish()?;

            finish_buffered_file(output)
        })
    }
}

/// Age identity decryptor for backup archives.
pub struct AgeIdentityDecryptor {
    identity: age::x25519::Identity,
}

impl AgeIdentityDecryptor {
    /// Parses an age X25519 identity string.
    ///
    /// # Errors
    ///
    /// Returns [`BackupError::Encryption`] when `identity` is not a valid age
    /// identity string.
    pub fn from_identity_string(identity: &str) -> BackupResult<Self> {
        let parsed_identity = identity
            .parse::<age::x25519::Identity>()
            .map_err(|error| encryption_error(&error))?;

        Ok(Self {
            identity: parsed_identity,
        })
    }
}

impl BackupDecryptor for AgeIdentityDecryptor {
    fn decrypt_in_place(&self, ciphertext_path: &Path, plaintext_path: &Path) -> BackupResult<()> {
        write_atomic_file(plaintext_path, |temp_file| {
            let input = BufReader::new(fs::File::open(ciphertext_path)?);
            let decryptor =
                age::Decryptor::new_buffered(input).map_err(|error| encryption_error(&error))?;
            let mut decrypted_input = decryptor
                .decrypt(std::iter::once(&self.identity as _))
                .map_err(|error| encryption_error(&error))?;
            let mut output = BufWriter::new(temp_file);
            io::copy(&mut decrypted_input, &mut output)?;

            finish_buffered_file(output)
        })
    }
}

fn write_atomic_file(
    path: &Path,
    write_temp: impl FnOnce(fs::File) -> BackupResult<()>,
) -> BackupResult<()> {
    let parent_dir = path.parent().ok_or_else(|| {
        BackupError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output path has no parent directory",
        ))
    })?;
    let temp_path = temp_output_path(path);
    let write_result = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp_path)
        .map_err(BackupError::from)
        .and_then(write_temp)
        .and_then(|()| fs::rename(&temp_path, path).map_err(BackupError::from))
        .and_then(|()| sync_directory(parent_dir));

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }

    write_result
}

fn finish_buffered_file(writer: BufWriter<fs::File>) -> BackupResult<()> {
    let file = writer
        .into_inner()
        .map_err(|error| BackupError::Io(error.into_error()))?;
    file.sync_all()?;

    Ok(())
}

fn recipient_fingerprint(recipient: &str) -> String {
    let digest = Sha256::digest(recipient.as_bytes());

    HEXLOWER.encode(&digest[..16])
}

fn temp_output_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backup-output");
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());

    path.with_file_name(format!(
        ".{file_name}.tmp-{}-{unique_suffix}",
        std::process::id()
    ))
}

fn sync_directory(path: &Path) -> BackupResult<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

fn encryption_error(error: &impl ToString) -> BackupError {
    BackupError::Encryption(error.to_string())
}
