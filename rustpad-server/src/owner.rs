//! Owner-only pages and endpoints behind Cloudflare Access: the document list
//! at `/documents` and its API under `/api/documents`.
//!
//! Every request needs a valid Cloudflare Access token (see [`crate::access`]).
//! Without an Access configuration or a database, everything here answers 404.
//! Requests that change something must send JSON, and a cross-site `Origin`
//! is refused, so a page on another site cannot drive these endpoints.

use std::sync::Arc;

use log::{error, warn};
use serde::Deserialize;
use warp::http::{HeaderValue, StatusCode, header};
use warp::reply::Response;
use warp::{Filter, Rejection, Reply, filters::BoxedFilter};

use crate::{ServerState, access, database::Database};

/// Longest title kept, in characters (the database column holds 255).
const MAX_TITLE_CHARS: usize = 200;

/// The request was not allowed (no or invalid token, or cross-site).
#[derive(Debug)]
struct Forbidden;
impl warp::reject::Reject for Forbidden {}

/// A server-side failure, already logged.
#[derive(Debug)]
struct Failed;
impl warp::reject::Reject for Failed {}

/// Checks the Access token and the request origin, and extracts the database.
fn authorized(state: ServerState) -> impl Filter<Extract = (Database,), Error = Rejection> + Clone {
    warp::header::optional::<String>(access::HEADER)
        .and(warp::header::optional::<String>("origin"))
        .and(warp::header::optional::<String>("host"))
        .and(warp::any().map(move || state.clone()))
        .and_then(
            |token: Option<String>,
             origin: Option<String>,
             host: Option<String>,
             state: ServerState| async move {
                let (Some(verifier), Some(db)) = (&state.access, &state.database) else {
                    return Err(warp::reject::not_found());
                };
                if let Some(origin) = origin {
                    let origin_host = origin.split("://").nth(1).unwrap_or_default();
                    if host.as_deref() != Some(origin_host) {
                        warn!("refused a cross-site request from {origin}");
                        return Err(warp::reject::custom(Forbidden));
                    }
                }
                let Some(token) = token else {
                    return Err(warp::reject::custom(Forbidden));
                };
                match verifier.verify(&token).await {
                    Ok(_) => Ok(db.clone()),
                    Err(e) => {
                        warn!("rejected an Access token: {e:#}");
                        Err(warp::reject::custom(Forbidden))
                    }
                }
            },
        )
}

/// Turns this module's rejections into responses; others pass through.
async fn recover(rejection: Rejection) -> Result<Response, Rejection> {
    if rejection.find::<Forbidden>().is_some() {
        Ok(warp::reply::with_status("Forbidden", StatusCode::FORBIDDEN).into_response())
    } else if rejection.find::<Failed>().is_some() {
        Ok(
            warp::reply::with_status("Server error", StatusCode::INTERNAL_SERVER_ERROR)
                .into_response(),
        )
    } else {
        Err(rejection)
    }
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn failed(e: anyhow::Error) -> Rejection {
    error!("owner request failed: {e:#}");
    warp::reject::custom(Failed)
}

/// The `/documents` page.
pub(crate) fn page(state: ServerState) -> BoxedFilter<(Response,)> {
    warp::path!("documents")
        .and(warp::get())
        .and(authorized(state))
        .and_then(|_db: Database| async move {
            match tokio::fs::read_to_string("dist/documents.html").await {
                Ok(html) => Ok::<_, Rejection>(no_store(warp::reply::html(html).into_response())),
                Err(_) => Err(warp::reject::not_found()),
            }
        })
        .recover(recover)
        .unify()
        .boxed()
}

/// The API under `/api/documents`.
pub(crate) fn api(state: ServerState) -> BoxedFilter<(Response,)> {
    let auth = authorized(state.clone());

    let list = warp::path::end()
        .and(warp::get())
        .and(auth.clone())
        .and_then(|db: Database| async move {
            let list = db.list().await.map_err(failed)?;
            Ok::<_, Rejection>(no_store(warp::reply::json(&list).into_response()))
        });

    let title = warp::path!(String / "title")
        .and(warp::put())
        .and(auth.clone())
        .and(warp::body::json())
        .and_then(set_title);

    let lock = warp::path!(String / "lock")
        .and(warp::post())
        .and(auth.clone())
        .and(with_state(state.clone()))
        .and(warp::body::json())
        .and_then(set_lock);

    let versions = warp::path!(String / "versions")
        .and(warp::get())
        .and(auth.clone())
        .and_then(|id: String, db: Database| async move {
            let versions = db.versions(&id).await.map_err(failed)?;
            Ok::<_, Rejection>(no_store(warp::reply::json(&versions).into_response()))
        });

    let version = warp::path!(String / "versions" / i64)
        .and(warp::get())
        .and(auth.clone())
        .and_then(|id: String, version_id: i64, db: Database| async move {
            match db.version(&id, version_id).await.map_err(failed)? {
                Some(version) => {
                    Ok::<_, Rejection>(no_store(warp::reply::json(&version).into_response()))
                }
                None => Ok(not_found()),
            }
        });

    let restore = warp::path!(String / "versions" / i64 / "restore")
        .and(warp::post())
        .and(auth.clone())
        .and(with_state(state.clone()))
        .and(warp::body::json())
        .and_then(restore);

    warp::path("documents")
        .and(
            list.or(title)
                .unify()
                .or(lock)
                .unify()
                .or(versions)
                .unify()
                .or(version)
                .unify()
                .or(restore)
                .unify(),
        )
        .recover(recover)
        .unify()
        .boxed()
}

fn with_state(
    state: ServerState,
) -> impl Filter<Extract = (ServerState,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || state.clone())
}

#[derive(Deserialize)]
struct LockRequest {
    locked: bool,
}

async fn set_lock(
    id: String,
    db: Database,
    state: ServerState,
    body: LockRequest,
) -> Result<Response, Rejection> {
    let rustpad = state.documents.get(&id).map(|d| Arc::clone(&d.rustpad));
    if let Some(rustpad) = &rustpad {
        // Save the open document first, so the stored text is the locked one.
        if rustpad.revision() > 0 || rustpad.language().is_some() {
            db.store(&id, &rustpad.snapshot()).await.map_err(failed)?;
        }
    }
    if !db.set_locked(&id, body.locked).await.map_err(failed)? {
        return Ok(not_found());
    }
    if body.locked {
        // Keep the locked text as a version.
        if let Some(document) = db.try_load(&id).await.map_err(failed)? {
            db.snapshot(&id, &document, "lock", state.versioning.keep_days)
                .await
                .map_err(failed)?;
            if let Some(rustpad) = &rustpad {
                rustpad.set_unversioned(false);
            }
        }
    }
    if let Some(rustpad) = rustpad {
        rustpad.set_locked(body.locked);
    }
    Ok(no_store(
        warp::reply::json(&serde_json::json!({ "locked": body.locked })).into_response(),
    ))
}

/// The body of a restore request; empty, but it must be JSON.
#[derive(Deserialize)]
struct RestoreRequest {}

async fn restore(
    id: String,
    version_id: i64,
    db: Database,
    state: ServerState,
    _body: RestoreRequest,
) -> Result<Response, Rejection> {
    let Some(version) = db.version(&id, version_id).await.map_err(failed)? else {
        return Ok(not_found());
    };
    let rustpad = crate::get_or_load(&state, &id).await?;
    // Keep the current text as a version first, so a restore can be undone.
    let current = rustpad.snapshot();
    db.store(&id, &current).await.map_err(failed)?;
    db.snapshot(&id, &current, "before restore", state.versioning.keep_days)
        .await
        .map_err(failed)?;
    rustpad
        .replace(&version.text, version.language.clone())
        .map_err(failed)?;
    db.store(&id, &rustpad.snapshot()).await.map_err(failed)?;
    Ok(no_store(
        warp::reply::json(&serde_json::json!({ "restored": version.id })).into_response(),
    ))
}

#[derive(Deserialize)]
struct TitleRequest {
    title: Option<String>,
}

async fn set_title(id: String, db: Database, body: TitleRequest) -> Result<Response, Rejection> {
    let title: String = body
        .title
        .unwrap_or_default()
        .trim()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    let title = (!title.is_empty()).then_some(title);
    if db.set_title(&id, title.as_deref()).await.map_err(failed)? {
        Ok(no_store(
            warp::reply::json(&serde_json::json!({ "title": title })).into_response(),
        ))
    } else {
        Ok(not_found())
    }
}

/// An explicit 404 (a rejection would merge with other routes' rejections).
fn not_found() -> Response {
    warp::reply::with_status("Not found", StatusCode::NOT_FOUND).into_response()
}
