use log::info;
use rustpad_server::{
    ServerConfig, access::AccessVerifier, database::Database, server_with_flusher,
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    pretty_env_logger::init();

    let port = std::env::var("PORT")
        .unwrap_or_else(|_| String::from("3030"))
        .parse()
        .expect("Unable to parse PORT");

    let mut config = ServerConfig {
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
        access: None,
    };

    if let (Ok(team), Ok(aud)) = (
        std::env::var("CF_ACCESS_TEAM_DOMAIN"),
        std::env::var("CF_ACCESS_AUD"),
    ) {
        if !team.trim().is_empty() && !aud.trim().is_empty() {
            let emails: Vec<String> = std::env::var("CF_ACCESS_ALLOWED_EMAILS")
                .unwrap_or_default()
                .split(',')
                .map(String::from)
                .collect();
            info!("document list enabled for Access team {}", team.trim());
            config.access = Some(AccessVerifier::new(&team, &aud, &emails));
        }
    }

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
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("failed to listen for SIGTERM");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.ok();
}
