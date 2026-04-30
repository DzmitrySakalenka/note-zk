#![allow(dead_code)]

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use axum_extra::extract::cookie::{Cookie, SameSite};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use zk_auth::{
    password::{HashParams, hash_password},
    session::{hash_csrf_token, hash_session_id},
};
use zk_domain::{AuthScope, LoginResponse};
use zk_server::{
    ServerError,
    config::ServerConfig,
    http::build_router,
    repo::{UserRepo, WebSessionInsert, WebSessionRepo},
    service::{self, CreateSessionSpec, IssueTokenSpec, SessionLifetime},
    state::{ServerState, bootstrap_with_config},
};

pub const SESSION_COOKIE_NAME: &str = "__Host-zk_session";
pub const CSRF_COOKIE_NAME: &str = "zk_csrf";
pub const CSRF_HEADER_NAME: &str = "X-CSRF-Token";
pub const USER_ID: &str = "user-a";
pub const USERNAME: &str = "alice";
pub const PASSWORD: &str = "correct horse battery";
pub const VALID_SESSION_ID: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
pub const VALID_CSRF_TOKEN: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

pub struct AuthHarness {
    _database_file: NamedTempFile,
    pub state: ServerState,
}

pub struct LoginFixture {
    pub session_cookie: Cookie<'static>,
    pub csrf_cookie: Cookie<'static>,
    pub csrf_token: String,
    pub cookie_header: String,
}

pub struct InsertedSession {
    pub session_id: String,
    pub csrf_token: String,
    pub cookie_header: String,
}

pub struct IssuedTokenFixture {
    pub token_id: String,
    pub plaintext: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCounts {
    pub total: i64,
    pub active: i64,
    pub revoked: i64,
}

#[derive(Clone, Copy, Default)]
pub struct RequestAuth<'a> {
    pub cookie_header: Option<&'a str>,
    pub csrf_token: Option<&'a str>,
    pub bearer_token: Option<&'a str>,
}

struct FixedSessionSpec {
    session_id: &'static str,
    csrf_token: &'static str,
    scopes: Vec<AuthScope>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl AuthHarness {
    pub fn new() -> Self {
        let database_file = NamedTempFile::new().expect("temp database file should be created");
        let state = bootstrap_with_config(test_config(database_file.path().to_path_buf()))
            .expect("bootstrap_with_config should succeed");

        Self {
            _database_file: database_file,
            state,
        }
    }

    pub fn with_default_user() -> Self {
        let harness = Self::new();
        harness.insert_default_user();
        harness
    }

    pub fn insert_default_user(&self) {
        self.insert_user(USER_ID, USERNAME, PASSWORD);
    }

    pub fn insert_user(&self, user_id: &str, username: &str, password: &str) {
        let password_hash = hash_password(
            password,
            HashParams {
                memory_kib: 8,
                iterations: 1,
                parallelism: 1,
            },
        )
        .expect("password hash fixture should be created");

        self.state
            .db()
            .with_conn(|connection| {
                UserRepo::create(
                    connection,
                    user_id,
                    username,
                    &password_hash,
                    parse_timestamp("2026-04-29T12:00:00Z"),
                )
            })
            .expect("user fixture insert should succeed");
    }

    pub async fn login_default(&self) -> LoginFixture {
        let response = build_router(self.state.clone())
            .oneshot(login_request(USERNAME, PASSWORD))
            .await
            .expect("login request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let session_cookie = response_cookie(&response, SESSION_COOKIE_NAME);
        let csrf_cookie = response_cookie(&response, CSRF_COOKIE_NAME);
        let body: LoginResponse = response_body(response).await;

        LoginFixture {
            cookie_header: cookie_header(&session_cookie, &csrf_cookie),
            session_cookie,
            csrf_cookie,
            csrf_token: body.csrf_token,
        }
    }

    pub fn issue_session(&self, scopes: Vec<AuthScope>) -> InsertedSession {
        let issued_session = self
            .state
            .db()
            .with_conn(|connection| {
                service::create_web_session(
                    connection,
                    &CreateSessionSpec {
                        user_id: USER_ID.to_string(),
                        scopes,
                        lifetime: SessionLifetime::default(),
                    },
                    Utc::now(),
                )
            })
            .expect("session fixture should be created");

        inserted_session(
            issued_session.plaintext_session_id,
            issued_session.plaintext_csrf_token,
        )
    }

    pub fn insert_expired_session(&self, scopes: Vec<AuthScope>) -> InsertedSession {
        let now = Utc::now();
        self.insert_fixed_session(FixedSessionSpec {
            session_id: VALID_SESSION_ID,
            csrf_token: VALID_CSRF_TOKEN,
            scopes,
            expires_at: now - ChronoDuration::minutes(1),
            revoked_at: None,
        })
    }

    pub fn insert_revoked_session(&self, scopes: Vec<AuthScope>) -> InsertedSession {
        let now = Utc::now();
        self.insert_fixed_session(FixedSessionSpec {
            session_id: VALID_SESSION_ID,
            csrf_token: VALID_CSRF_TOKEN,
            scopes,
            expires_at: now + ChronoDuration::hours(1),
            revoked_at: Some(now - ChronoDuration::minutes(1)),
        })
    }

    pub fn issue_device_token(&self, scopes: Vec<AuthScope>) -> IssuedTokenFixture {
        let issued = self
            .state
            .db()
            .with_conn(|connection| {
                service::issue_device_token(
                    connection,
                    &IssueTokenSpec {
                        label: "Sync token".to_string(),
                        device_id: None,
                        scopes,
                    },
                    Utc::now(),
                )
            })
            .expect("device token fixture should be issued");

        IssuedTokenFixture {
            token_id: issued.token_id,
            plaintext: issued.plaintext,
        }
    }

    pub fn issue_revoked_device_token(&self, scopes: Vec<AuthScope>) -> IssuedTokenFixture {
        let issued = self.issue_device_token(scopes);
        self.state
            .db()
            .with_conn(|connection| {
                service::revoke_device_token(connection, &issued.token_id, Utc::now())
            })
            .expect("device token fixture should be revoked");
        issued
    }

    pub fn session_counts_for_default_user(&self) -> SessionCounts {
        self.state
            .db()
            .with_conn(|connection| {
                connection
                    .query_row(
                        r"
SELECT
    COUNT(*),
    COALESCE(SUM(CASE WHEN revoked_at IS NULL THEN 1 ELSE 0 END), 0),
    COALESCE(SUM(CASE WHEN revoked_at IS NOT NULL THEN 1 ELSE 0 END), 0)
FROM web_sessions
WHERE user_id = ?1
",
                        [USER_ID],
                        |row| {
                            Ok(SessionCounts {
                                total: row.get(0)?,
                                active: row.get(1)?,
                                revoked: row.get(2)?,
                            })
                        },
                    )
                    .map_err(ServerError::Database)
            })
            .expect("session counts should be readable")
    }

    fn insert_fixed_session(&self, spec: FixedSessionSpec) -> InsertedSession {
        let created_at = Utc::now() - ChronoDuration::hours(2);
        let session_id_sha256_hex = hash_session_id(spec.session_id);

        self.state
            .db()
            .with_conn(|connection| {
                WebSessionRepo::create(
                    connection,
                    &WebSessionInsert {
                        session_id_sha256_hex: session_id_sha256_hex.clone(),
                        user_id: USER_ID.to_string(),
                        csrf_token_sha256_hex: hash_csrf_token(spec.csrf_token),
                        scopes: spec.scopes,
                        created_at,
                        expires_at: spec.expires_at,
                    },
                )?;

                if let Some(revoked_at) = spec.revoked_at {
                    WebSessionRepo::revoke(connection, &session_id_sha256_hex, revoked_at)?;
                }

                Ok(())
            })
            .expect("fixed session fixture should be inserted");

        inserted_session(spec.session_id.to_string(), spec.csrf_token.to_string())
    }
}

pub fn json_request(
    method: Method,
    uri: &str,
    body: &Value,
    auth: RequestAuth<'_>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");

    if let Some(cookie_header) = auth.cookie_header {
        builder = builder.header(header::COOKIE, cookie_header);
    }

    if let Some(csrf_token) = auth.csrf_token {
        builder = builder.header(CSRF_HEADER_NAME, csrf_token);
    }

    if let Some(bearer_token) = auth.bearer_token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {bearer_token}"));
    }

    builder
        .body(Body::from(body.to_string()))
        .expect("request should be built")
}

pub fn login_request(username: &str, password: &str) -> Request<Body> {
    json_request(
        Method::POST,
        "/auth/login",
        &json!({
            "username": username,
            "password": password,
        }),
        RequestAuth::default(),
    )
}

pub async fn response_json(response: axum::response::Response) -> Value {
    let body_bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    serde_json::from_slice(&body_bytes).expect("response body should be valid json")
}

pub async fn response_body<T: serde::de::DeserializeOwned>(
    response: axum::response::Response,
) -> T {
    serde_json::from_value(response_json(response).await).expect("response body should deserialize")
}

pub async fn assert_json_error(
    response: axum::response::Response,
    status: StatusCode,
    code: &str,
    message: &str,
) {
    assert_eq!(response.status(), status);
    let body = response_json(response).await;

    assert_eq!(body["error"]["code"], code);
    assert_eq!(body["error"]["message"], message);
}

pub fn set_cookie_count(response: &axum::response::Response) -> usize {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .count()
}

pub fn response_cookie(response: &axum::response::Response, name: &str) -> Cookie<'static> {
    // Parse each Set-Cookie header independently so attribute order stays irrelevant.
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| Cookie::parse_encoded(value.to_string()).ok())
        .find(|cookie| cookie.name() == name)
        .unwrap_or_else(|| panic!("{name} cookie should be present"))
}

pub fn assert_session_cookie_contract(cookie: &Cookie<'_>) {
    assert_eq!(cookie.name(), SESSION_COOKIE_NAME);
    assert!(!cookie.value().is_empty());
    assert_eq!(cookie.path(), Some("/"));
    assert_eq!(cookie.domain(), None);
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.http_only(), Some(true));
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(
        cookie
            .max_age()
            .expect("session cookie should have max age")
            .whole_seconds(),
        86_400
    );
}

pub fn assert_csrf_cookie_contract(cookie: &Cookie<'_>, expected_value: &str) {
    assert_eq!(cookie.name(), CSRF_COOKIE_NAME);
    assert_eq!(cookie.value(), expected_value);
    assert_eq!(cookie.path(), Some("/"));
    assert_eq!(cookie.domain(), None);
    assert_eq!(cookie.secure(), Some(true));
    assert_eq!(cookie.http_only(), None);
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(
        cookie
            .max_age()
            .expect("CSRF cookie should have max age")
            .whole_seconds(),
        86_400
    );
}

fn test_config(database_path: PathBuf) -> ServerConfig {
    ServerConfig {
        database_path,
        listen_addr: "127.0.0.1:7878"
            .parse::<SocketAddr>()
            .expect("test listen addr should parse"),
        log_filter: "info,zk_server=debug".to_string(),
        stub_auth_token_sha256_hex: None,
        trash_retention_days: 14,
        max_batch_size: 1000,
        backup: None,
    }
}

fn parse_timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .expect("timestamp fixture should parse")
        .with_timezone(&Utc)
}

fn inserted_session(session_id: String, csrf_token: String) -> InsertedSession {
    InsertedSession {
        cookie_header: format!(
            "{SESSION_COOKIE_NAME}={session_id}; {CSRF_COOKIE_NAME}={csrf_token}"
        ),
        session_id,
        csrf_token,
    }
}

fn cookie_header(session_cookie: &Cookie<'_>, csrf_cookie: &Cookie<'_>) -> String {
    format!(
        "{}={}; {}={}",
        session_cookie.name(),
        session_cookie.value(),
        csrf_cookie.name(),
        csrf_cookie.value()
    )
}
