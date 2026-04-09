use std::{env, net::SocketAddr, sync::Arc};

use kou_router::{build_app, init_db, routes::AppState, SqliteRepository};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kou_router=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url = env::var("KOU_ROUTER_DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://kou-router.db".to_string());
    let bind = env::var("KOU_ROUTER_BIND").unwrap_or_else(|_| "0.0.0.0:20128".to_string());

    let pool = init_db(&database_url).await?;
    let repository = Arc::new(SqliteRepository::new(pool));
    let state = AppState::new(repository);
    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let addr: SocketAddr = listener.local_addr()?;
    tracing::info!(%addr, "kou-router listening");

    axum::serve(listener, app).await?;
    Ok(())
}
