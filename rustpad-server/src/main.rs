use log::info;
use rustpad_server::{database::Database, server_with_flusher, ServerConfig};

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    pretty_env_logger::init();

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| String::from("3030"))
        .parse()
        .expect("Unable to parse PORT");

    let config = ServerConfig {
        expiry_days: std::env::var("EXPIRY_DAYS")
            .unwrap_or_else(|_| String::from("1"))
            .parse()
            .expect("Unable to parse EXPIRY_DAYS"),
        database: match std::env::var("DATABASE_URL").or_else(|_| std::env::var("SQLITE_URI")) {
            Ok(uri) => Some(
                Database::new(&uri)
                    .await
                    .expect("Unable to connect to DATABASE_URL / SQLITE_URI"),
            ),
            Err(_) => None,
        },
    };

    let (filter, flusher) = server_with_flusher(config);
    tokio::select! {
        _ = warp::serve(filter).run(([0, 0, 0, 0], port)) => {}
        _ = shutdown_signal() => {
            let stored = flusher.flush().await;
            info!("shutting down, saved {} documents", stored);
        }
    }
}

/// Waits for SIGTERM (sent by `docker stop` and Kubernetes) or Ctrl-C.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("failed to listen for SIGTERM");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.ok();
}
