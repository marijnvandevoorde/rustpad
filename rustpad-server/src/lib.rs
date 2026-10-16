//! Server backend for the Rustpad collaborative text editor.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use dashmap::DashMap;
use log::{error, info, warn};
use rand::RngExt;
use serde::Serialize;
use tokio::time::{self, Instant};
use warp::{Filter, Rejection, Reply, filters::BoxedFilter, ws::Ws};

use crate::{access::AccessVerifier, database::Database, rustpad::Rustpad};

pub mod access;
pub mod database;
mod ot;
mod owner;
mod rustpad;

/// An entry stored in the global server map.
///
/// Each entry corresponds to a single document. This is garbage collected by a
/// background task after one day of inactivity, to avoid server memory usage
/// growing without bound.
struct Document {
    last_accessed: Instant,
    rustpad: Arc<Rustpad>,
}

impl Document {
    fn new(rustpad: Arc<Rustpad>) -> Self {
        Self {
            last_accessed: Instant::now(),
            rustpad,
        }
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        self.rustpad.kill();
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct CustomReject(anyhow::Error);

impl warp::reject::Reject for CustomReject {}

/// The shared state of the server, accessible from within request handlers.
#[derive(Clone)]
struct ServerState {
    /// Concurrent map storing in-memory documents.
    documents: Arc<DashMap<String, Document>>,
    /// Connection to the database pool, if persistence is enabled.
    database: Option<Database>,
    /// Cloudflare Access verifier, if the document list is enabled.
    access: Option<AccessVerifier>,
}

/// Statistics about the server, returned from an API endpoint.
#[derive(Serialize)]
struct Stats {
    /// System time when the server started, in seconds since Unix epoch.
    start_time: u64,
    /// Number of documents currently tracked by the server.
    num_documents: usize,
    /// Number of documents persisted in the database.
    database_size: usize,
}

/// Server configuration.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Number of days to clean up documents after inactivity.
    pub expiry_days: u32,
    /// Database object, for persistence if desired.
    pub database: Option<Database>,
    /// Enables the private document list (`/documents`), for users that pass
    /// this Cloudflare Access check. Needs a database.
    pub access: Option<AccessVerifier>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            expiry_days: 1,
            database: None,
            access: None,
        }
    }
}

/// A combined filter handling all server routes.
pub fn server(config: ServerConfig) -> BoxedFilter<(impl Reply,)> {
    server_with_flusher(config).0
}

/// Like [`server`], plus a [`Flusher`] to save every document before exiting.
pub fn server_with_flusher(config: ServerConfig) -> (BoxedFilter<(impl Reply,)>, Flusher) {
    let (backend, state) = backend(config);
    let filter = warp::path("api")
        .and(backend)
        .or(owner::page(state.clone()))
        .or(frontend())
        .boxed();
    (filter, Flusher(state))
}

/// Saves all in-memory documents to the database, e.g. on shutdown, so edits
/// made since the last periodic save are not lost.
#[derive(Clone)]
pub struct Flusher(ServerState);

impl Flusher {
    /// Disconnects all clients and stores every edited document. Returns the
    /// number of documents stored.
    pub async fn flush(&self) -> usize {
        let Some(db) = &self.0.database else {
            return 0;
        };
        let documents: Vec<(String, Arc<Rustpad>)> = self
            .0
            .documents
            .iter()
            .map(|entry| (entry.key().clone(), Arc::clone(&entry.rustpad)))
            .collect();
        let mut stored = 0;
        for (id, rustpad) in documents {
            // Stop accepting edits, so nothing changes after the snapshot.
            rustpad.kill();
            // A document that was never edited and has no language is empty.
            if rustpad.revision() == 0 && rustpad.language().is_none() {
                continue;
            }
            match db.store(&id, &rustpad.snapshot()).await {
                Ok(()) => stored += 1,
                Err(e) => error!("when flushing document {}: {}", id, e),
            }
        }
        stored
    }
}

/// Construct routes for static files from React.
fn frontend() -> BoxedFilter<(impl Reply,)> {
    warp::fs::dir("dist").boxed()
}

/// Construct backend routes, including WebSocket handlers.
fn backend(config: ServerConfig) -> (BoxedFilter<(impl Reply,)>, ServerState) {
    let state = ServerState {
        documents: Default::default(),
        database: config.database,
        access: config.access,
    };
    tokio::spawn(cleaner(state.clone(), config.expiry_days));

    let returned_state = state.clone();
    let state_filter = warp::any().map(move || state.clone());

    let socket = warp::path!("socket" / String)
        .and(warp::ws())
        .and(state_filter.clone())
        .and_then(socket_handler);

    let text = warp::path!("text" / String)
        .and(state_filter.clone())
        .and_then(text_handler);

    let start_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("SystemTime returned before UNIX_EPOCH")
        .as_secs();
    let stats = warp::path!("stats")
        .and(warp::any().map(move || start_time))
        .and(state_filter.clone())
        .and_then(stats_handler);

    let documents = owner::api(returned_state.clone());

    (
        socket.or(text).or(stats).or(documents).boxed(),
        returned_state,
    )
}

/// Handler for the `/api/socket/{id}` endpoint.
async fn socket_handler(id: String, ws: Ws, state: ServerState) -> Result<impl Reply, Rejection> {
    let rustpad = get_or_load(&state, &id).await?;
    Ok(ws.on_upgrade(|socket| async move { rustpad.on_connection(socket).await }))
}

/// Returns a document from memory, loading it from the database if needed.
async fn get_or_load(state: &ServerState, id: &str) -> Result<Arc<Rustpad>, Rejection> {
    use dashmap::mapref::entry::Entry;

    let id = id.to_owned();
    let mut entry = match state.documents.entry(id.clone()) {
        Entry::Occupied(e) => e.into_ref(),
        Entry::Vacant(e) => {
            // A database error must not open the document as empty: the
            // persister would then overwrite the stored text.
            let rustpad = Arc::new(match &state.database {
                Some(db) => match db.try_load(&id).await {
                    Ok(document) => document.map(Rustpad::from).unwrap_or_default(),
                    Err(e) => {
                        error!("when loading document {}: {}", id, e);
                        return Err(warp::reject::custom(CustomReject(e)));
                    }
                },
                None => Rustpad::default(),
            });
            if let Some(db) = &state.database {
                tokio::spawn(persister(id, Arc::clone(&rustpad), db.clone()));
            }
            e.insert(Document::new(rustpad))
        }
    };

    let value = entry.value_mut();
    value.last_accessed = Instant::now();
    Ok(Arc::clone(&value.rustpad))
}

/// Handler for the `/api/text/{id}` endpoint.
async fn text_handler(id: String, state: ServerState) -> Result<impl Reply, Rejection> {
    Ok(match state.documents.get(&id) {
        Some(value) => value.rustpad.text(),
        None => {
            if let Some(db) = &state.database {
                match db.try_load(&id).await {
                    Ok(document) => document.map(|d| d.text).unwrap_or_default(),
                    Err(e) => return Err(warp::reject::custom(CustomReject(e))),
                }
            } else {
                String::new()
            }
        }
    })
}

/// Handler for the `/api/stats` endpoint.
async fn stats_handler(start_time: u64, state: ServerState) -> Result<impl Reply, Rejection> {
    let num_documents = state.documents.len();
    let database_size = match state.database {
        None => 0,
        Some(db) => match db.count().await {
            Ok(size) => size,
            Err(e) => return Err(warp::reject::custom(CustomReject(e))),
        },
    };
    Ok(warp::reply::json(&Stats {
        start_time,
        num_documents,
        database_size,
    }))
}

const HOUR: Duration = Duration::from_secs(3600);

/// Reclaims memory for documents.
async fn cleaner(state: ServerState, expiry_days: u32) {
    loop {
        time::sleep(HOUR).await;
        let mut keys = Vec::new();
        for entry in &*state.documents {
            if entry.last_accessed.elapsed() > HOUR * 24 * expiry_days {
                keys.push(entry.key().clone());
            }
        }
        info!("cleaner removing keys: {:?}", keys);
        for key in keys {
            state.documents.remove(&key);
        }
    }
}

const PERSIST_INTERVAL: Duration = Duration::from_secs(3);
const PERSIST_INTERVAL_JITTER: Duration = Duration::from_secs(1);

/// Persists changed documents after a fixed time interval.
async fn persister(id: String, rustpad: Arc<Rustpad>, db: Database) {
    // Start from the state loaded from the database, which needs no write.
    let mut last_revision = rustpad.revision();
    let mut last_language = rustpad.language();
    while !rustpad.killed() {
        let interval =
            PERSIST_INTERVAL + rand::rng().random_range(Duration::ZERO..=PERSIST_INTERVAL_JITTER);
        time::sleep(interval).await;
        let revision = rustpad.revision();
        // A language change does not create a revision, so compare it too.
        if revision > last_revision || rustpad.language() != last_language {
            info!("persisting revision {} for id = {}", revision, id);
            let document = rustpad.snapshot();
            if let Err(e) = db.store(&id, &document).await {
                error!("when persisting document {}: {}", id, e);
            } else {
                last_revision = revision;
                last_language = document.language;
            }
        }
    }
}
