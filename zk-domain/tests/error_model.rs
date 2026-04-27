// Integration tests are separate crates; each uses only a subset of package dependencies.
#![allow(unused_crate_dependencies)]

use zk_domain::error::{DomainError, Result};

// ==========================================================================
// DomainError -- Display messages
// ==========================================================================

#[test]
fn invalid_id_display() {
    let err = DomainError::InvalidId {
        value: "not-a-ulid!!!".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("not-a-ulid!!!"), "message: {msg}");
}

#[test]
fn invalid_path_display() {
    let err = DomainError::InvalidPath {
        path: "../escape/attempt".to_string(),
        reason: "path traversal not allowed".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("../escape/attempt"), "message: {msg}");
    assert!(msg.contains("path traversal not allowed"), "message: {msg}");
}

#[test]
fn invalid_frontmatter_display() {
    let err = DomainError::InvalidFrontmatter {
        path: "notes/broken.md".to_string(),
        reason: "YAML parse error at line 3".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("notes/broken.md"), "message: {msg}");
    assert!(msg.contains("YAML parse error at line 3"), "message: {msg}");
}

#[test]
fn missing_required_field_display() {
    let err = DomainError::MissingRequiredField { field: "title" };
    let msg = err.to_string();
    assert!(msg.contains("title"), "message: {msg}");
}

// ==========================================================================
// DomainError -- trait implementations
// ==========================================================================

#[test]
fn domain_error_is_std_error() {
    let err = DomainError::InvalidId {
        value: "bad".to_string(),
    };
    // std::error::Error is implemented (thiserror provides this).
    let _: &dyn std::error::Error = &err;
}

#[test]
fn domain_error_debug() {
    let err = DomainError::InvalidPath {
        path: "x".to_string(),
        reason: "y".to_string(),
    };
    let debug = format!("{err:?}");
    assert!(!debug.is_empty());
}

#[test]
fn domain_error_clone() {
    let err = DomainError::MissingRequiredField { field: "tags" };
    let err2 = err.clone();
    assert_eq!(err.to_string(), err2.to_string());
}

// ==========================================================================
// Result alias
// ==========================================================================

#[test]
fn result_alias_ok() {
    let r: Result<i32> = Ok(42);
    assert!(matches!(r, Ok(42)));
}

#[test]
fn result_alias_err() {
    let r: Result<i32> = Err(DomainError::MissingRequiredField { field: "id" });
    assert!(r.is_err());
}

// ==========================================================================
// Each variant is distinct
// ==========================================================================

#[test]
fn different_variants_produce_different_messages() {
    let e1 = DomainError::InvalidId {
        value: "x".to_string(),
    };
    let e2 = DomainError::InvalidPath {
        path: "x".to_string(),
        reason: "y".to_string(),
    };
    let e3 = DomainError::InvalidFrontmatter {
        path: "x".to_string(),
        reason: "y".to_string(),
    };
    let e4 = DomainError::MissingRequiredField { field: "z" };

    let msgs: Vec<String> = vec![e1, e2, e3, e4]
        .into_iter()
        .map(|e| e.to_string())
        .collect();
    // All messages should be unique.
    for i in 0..msgs.len() {
        for j in (i + 1)..msgs.len() {
            assert_ne!(
                msgs[i], msgs[j],
                "variants {i} and {j} have the same message"
            );
        }
    }
}

// ==========================================================================
// Error messages are human-readable (not raw debug dumps)
// ==========================================================================

#[test]
fn error_messages_are_human_readable() {
    let cases: Vec<DomainError> = vec![
        DomainError::InvalidId {
            value: "oops".to_string(),
        },
        DomainError::InvalidPath {
            path: "bad/path".to_string(),
            reason: "contains null byte".to_string(),
        },
        DomainError::InvalidFrontmatter {
            path: "notes/test.md".to_string(),
            reason: "missing delimiter".to_string(),
        },
        DomainError::MissingRequiredField { field: "title" },
    ];
    for err in &cases {
        let msg = err.to_string();
        // Should read as a sentence/phrase, not contain Rust debug formatting artifacts.
        assert!(!msg.contains("InvalidId"), "raw variant name leaked: {msg}");
        assert!(
            !msg.contains("InvalidPath"),
            "raw variant name leaked: {msg}"
        );
        assert!(
            !msg.contains("InvalidFrontmatter"),
            "raw variant name leaked: {msg}"
        );
        assert!(
            !msg.contains("MissingRequiredField"),
            "raw variant name leaked: {msg}"
        );
        assert!(!msg.is_empty(), "empty error message");
    }
}
