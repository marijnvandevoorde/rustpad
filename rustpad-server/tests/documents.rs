//! Tests for the private document list behind Cloudflare Access.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use common::*;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use operational_transform::OperationSeq;
use rustpad_server::{
    ServerConfig,
    access::AccessVerifier,
    database::{Database, PersistedDocument},
    server,
};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use warp::{Reply, filters::BoxedFilter};

pub mod common;

const TEAM: &str = "example.cloudflareaccess.com";
const AUD: &str = "test-audience-tag";
const KEY: &str = include_str!("fixtures/access-test-key.pem");
const OTHER_KEY: &str = include_str!("fixtures/other-test-key.pem");
const JWKS: &str = include_str!("fixtures/access-test-jwks.json");

/// Serves the test key set over HTTP, counting the requests.
async fn serve_jwks() -> Result<(String, Arc<AtomicUsize>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/cdn-cgi/access/certs", listener.local_addr()?);
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0; 4096];
            let _ = socket.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                JWKS.len(),
                JWKS
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    Ok((url, count))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn token(key: &str, kid: &str, claims: Value) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.into());
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(key.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn claims(email: &str) -> Value {
    json!({
        "aud": [AUD],
        "iss": format!("https://{TEAM}"),
        "exp": now() + 300,
        "iat": now(),
        "email": email,
    })
}

async fn database() -> Result<Database> {
    // Kept on disk: the pool opens new connections to it later.
    let path = NamedTempFile::new()?.into_temp_path().keep()?;
    let database = Database::new(&format!("sqlite://{}", path.to_str().unwrap())).await?;
    database
        .store(
            "abc123",
            &PersistedDocument {
                text: "fn main() {}".into(),
                language: Some("rust".into()),
            },
        )
        .await?;
    database.set_title("abc123", Some("Interview Jane")).await?;
    Ok(database)
}

async fn get(
    filter: &BoxedFilter<(impl Reply + 'static,)>,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, String) {
    let mut request = warp::test::request().path(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let resp = request.reply(filter).await;
    (
        resp.status().as_u16(),
        String::from_utf8_lossy(resp.body()).into_owned(),
    )
}

#[tokio::test]
async fn test_list_disabled_without_access() -> Result<()> {
    let filter = server(ServerConfig {
        database: Some(database().await?),
        ..Default::default()
    });
    assert_eq!(get(&filter, "/api/documents", &[]).await.0, 404);
    assert_eq!(get(&filter, "/documents", &[]).await.0, 404);
    // The editor is unaffected.
    assert_eq!(
        get(&filter, "/api/text/abc123", &[]).await.1,
        "fn main() {}"
    );
    Ok(())
}

#[tokio::test]
async fn test_list_with_access() -> Result<()> {
    pretty_env_logger::try_init().ok();
    let (certs_url, fetches) = serve_jwks().await?;
    let allowed = vec!["Owner@Example.com".to_string()];
    let filter = server(ServerConfig {
        database: Some(database().await?),
        access: Some(AccessVerifier::with_certs_url(
            TEAM, AUD, &allowed, &certs_url,
        )),
        ..Default::default()
    });
    let header = "cf-access-jwt-assertion";

    // No token, or only the unsigned email header: denied.
    assert_eq!(get(&filter, "/api/documents", &[]).await.0, 403);
    assert_eq!(get(&filter, "/documents", &[]).await.0, 403);
    let unsigned = [("cf-access-authenticated-user-email", "owner@example.com")];
    assert_eq!(get(&filter, "/api/documents", &unsigned).await.0, 403);

    // A valid token for an allowed user (emails compare case-insensitively).
    let good = token(KEY, "test-key", claims("owner@example.com"));
    let (status, body) = get(&filter, "/api/documents", &[(header, &good)]).await;
    assert_eq!(status, 200);
    let list: Value = serde_json::from_str(&body)?;
    assert_eq!(list[0]["id"], "abc123");
    assert_eq!(list[0]["title"], "Interview Jane");
    assert_eq!(list[0]["language"], "rust");
    assert_eq!(list[0]["preview"], "fn main() {}");
    assert!(list[0]["updated_at"].is_string());

    let bad_tokens = [
        // Someone else's account.
        token(KEY, "test-key", claims("intruder@example.com")),
        // No email.
        token(KEY, "test-key", {
            let mut c = claims("x");
            c.as_object_mut().unwrap().remove("email");
            c
        }),
        // Another Access application.
        token(KEY, "test-key", {
            let mut c = claims("owner@example.com");
            c["aud"] = json!(["another-app"]);
            c
        }),
        // Another Access team.
        token(KEY, "test-key", {
            let mut c = claims("owner@example.com");
            c["iss"] = json!("https://evil.cloudflareaccess.com");
            c
        }),
        // Expired.
        token(KEY, "test-key", {
            let mut c = claims("owner@example.com");
            c["exp"] = json!(now() - 3600);
            c
        }),
        // Signed with a key that is not in the key set, under a known key id.
        token(OTHER_KEY, "test-key", claims("owner@example.com")),
        // Unknown key id.
        token(KEY, "unknown-key", claims("owner@example.com")),
        // HS256 signed with the public key set as a shared secret.
        encode(
            &{
                let mut h = Header::new(Algorithm::HS256);
                h.kid = Some("test-key".into());
                h
            },
            &claims("owner@example.com"),
            &EncodingKey::from_secret(JWKS.as_bytes()),
        )?,
        // Not a token at all.
        "garbage".to_string(),
    ];
    for bad in &bad_tokens {
        assert_eq!(
            get(&filter, "/api/documents", &[(header, bad)]).await.0,
            403,
            "token should be rejected: {bad}"
        );
    }

    // The key set is cached; an unknown key id refetches it at most once a
    // minute, so all of the above took a single fetch.
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn test_cloudflare_key_set_format() {
    // Cloudflare's certs endpoint adds public_cert(s) next to the keys.
    let mut body: Value = serde_json::from_str(JWKS).unwrap();
    body["public_cert"] = json!({ "kid": "test-key", "cert": "-----BEGIN CERTIFICATE-----" });
    body["public_certs"] = json!([body["public_cert"].clone()]);
    let set: jsonwebtoken::jwk::JwkSet = serde_json::from_value(body).unwrap();
    assert!(set.find("test-key").is_some());
}

/// Sends a request with a method, headers and an optional JSON body.
async fn send(
    filter: &BoxedFilter<(impl Reply + 'static,)>,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    json_body: Option<Value>,
) -> (u16, String) {
    let mut request = warp::test::request().method(method).path(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if let Some(body) = json_body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let resp = request.reply(filter).await;
    (
        resp.status().as_u16(),
        String::from_utf8_lossy(resp.body()).into_owned(),
    )
}

/// A server with Access enabled, and a valid token for it.
async fn owner_server() -> Result<(BoxedFilter<(impl Reply,)>, Database, String)> {
    let (certs_url, _) = serve_jwks().await?;
    let database = database().await?;
    let filter = server(ServerConfig {
        database: Some(database.clone()),
        access: Some(AccessVerifier::with_certs_url(TEAM, AUD, &[], &certs_url)),
        ..Default::default()
    });
    Ok((
        filter,
        database,
        token(KEY, "test-key", claims("owner@example.com")),
    ))
}

#[tokio::test]
async fn test_set_title() -> Result<()> {
    let (filter, database, good) = owner_server().await?;
    let auth = [("cf-access-jwt-assertion", good.as_str())];
    let path = "/api/documents/abc123/title";

    let (status, body) = send(
        &filter,
        "PUT",
        path,
        &auth,
        Some(json!({ "title": "  Interview Bob " })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        database.list().await?[0].title.as_deref(),
        Some("Interview Bob")
    );

    // Same-site requests are fine; cross-site ones are refused.
    let same_site = [
        ("cf-access-jwt-assertion", good.as_str()),
        ("host", "code.example.com"),
        ("origin", "https://code.example.com"),
    ];
    let cross_site = [
        ("cf-access-jwt-assertion", good.as_str()),
        ("host", "code.example.com"),
        ("origin", "https://evil.example.net"),
    ];
    assert_eq!(
        send(
            &filter,
            "PUT",
            path,
            &same_site,
            Some(json!({ "title": "A" }))
        )
        .await
        .0,
        200
    );
    assert_eq!(
        send(
            &filter,
            "PUT",
            path,
            &cross_site,
            Some(json!({ "title": "B" }))
        )
        .await
        .0,
        403
    );

    // No token, or not JSON (a plain form post): refused.
    assert_eq!(
        send(&filter, "PUT", path, &[], Some(json!({ "title": "C" })))
            .await
            .0,
        403
    );
    let resp = warp::test::request()
        .method("PUT")
        .path(path)
        .header("cf-access-jwt-assertion", good.as_str())
        .header("content-type", "application/x-www-form-urlencoded")
        .body("title=D")
        .reply(&filter)
        .await;
    assert!(resp.status().is_client_error());
    assert_eq!(database.list().await?[0].title.as_deref(), Some("A"));

    // Clearing, and a document that does not exist.
    assert_eq!(
        send(&filter, "PUT", path, &auth, Some(json!({ "title": "  " })))
            .await
            .0,
        200
    );
    assert_eq!(database.list().await?[0].title, None);
    assert_eq!(
        send(
            &filter,
            "PUT",
            "/api/documents/nope/title",
            &auth,
            Some(json!({ "title": "x" }))
        )
        .await
        .0,
        404
    );
    Ok(())
}

/// Reads the messages a client gets on connecting, up to and including
/// `last` (a message key such as "Locked").
async fn initial(client: &mut JsonSocket, last: &str) -> Result<Vec<Value>> {
    let mut messages = Vec::new();
    loop {
        let msg = client.recv().await?;
        let done = msg.get(last).is_some();
        messages.push(msg);
        if done {
            return Ok(messages);
        }
    }
}

/// Waits (up to 5 s) for a message with this key, skipping others.
async fn recv_key(client: &mut JsonSocket, key: &str) -> Result<Value> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let msg = client.recv().await?;
            if msg.get(key).is_some() {
                return Ok(msg);
            }
        }
    })
    .await?
}

/// Waits (up to 5 s) for the server to close the socket, skipping messages.
async fn expect_closed(client: &mut JsonSocket) -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while client.recv().await.is_ok() {}
    })
    .await?;
    Ok(())
}

fn insert(revision: usize, base_len: u64, text: &str) -> Value {
    let mut operation = OperationSeq::default();
    operation.retain(base_len);
    operation.insert(text);
    json!({ "Edit": { "revision": revision, "operation": operation } })
}

#[tokio::test]
async fn test_lock() -> Result<()> {
    pretty_env_logger::try_init().ok();
    let (filter, database, good) = owner_server().await?;
    let auth = [("cf-access-jwt-assertion", good.as_str())];
    let lock = "/api/documents/abc123/lock";

    let mut alice = connect(&filter, "abc123").await?;
    initial(&mut alice, "Language").await?;
    let mut bob = connect(&filter, "abc123").await?;
    initial(&mut bob, "Language").await?;

    // Locking reaches every connected editor.
    let (status, body) = send(
        &filter,
        "POST",
        lock,
        &auth,
        Some(json!({ "locked": true })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(alice.recv().await?, json!({ "Locked": true }));
    assert_eq!(bob.recv().await?, json!({ "Locked": true }));
    assert!(database.list().await?[0].locked);

    // Edits and language changes are refused (the socket is closed).
    alice.send(&insert(1, 12, "!")).await;
    expect_closed(&mut alice).await?;
    bob.send(&json!({ "SetLanguage": "python" })).await;
    expect_closed(&mut bob).await?;
    expect_text(&filter, "abc123", "fn main() {}").await;

    // Reconnecting clients, and a restarted server, see it locked.
    let mut carol = connect(&filter, "abc123").await?;
    let messages = initial(&mut carol, "Locked").await?;
    assert!(messages.contains(&json!({ "Language": "rust" })));
    let restarted = server(ServerConfig {
        database: Some(database.clone()),
        ..Default::default()
    });
    let mut dave = connect(&restarted, "abc123").await?;
    initial(&mut dave, "Locked").await?;

    // Unlocking makes it editable again.
    let (status, _) = send(
        &filter,
        "POST",
        lock,
        &auth,
        Some(json!({ "locked": false })),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(recv_key(&mut carol, "Locked").await?, json!({ "Locked": false }));
    carol.send(&insert(1, 12, "!")).await;
    recv_key(&mut carol, "History").await?;
    expect_text(&filter, "abc123", "fn main() {}!").await;
    assert!(!database.list().await?[0].locked);

    // Owner-only, and only for existing documents.
    assert_eq!(
        send(&filter, "POST", lock, &[], Some(json!({ "locked": true })))
            .await
            .0,
        403
    );
    assert_eq!(
        send(
            &filter,
            "POST",
            "/api/documents/nope/lock",
            &auth,
            Some(json!({ "locked": true }))
        )
        .await
        .0,
        404
    );
    Ok(())
}
