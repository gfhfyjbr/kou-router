use std::{env, io::IsTerminal, net::SocketAddr, sync::Arc};

use kou_router::{
    SqliteRepository, auth::bootstrap, build_app, build_headless_app, init_db, routes::AppState,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const HELP: &str = "\
kou-router — multi-protocol LLM router

USAGE:
    kou-router [serve] [OPTIONS]    web UI + API (default)
    kou-router daemon [OPTIONS]     headless API daemon (no web UI)

OPTIONS:
    --bind <ADDR>    listen address
                     [serve: 0.0.0.0:20128, daemon: 127.0.0.1:20128]
    --db <URL>       SQLite DSN [default: sqlite://kou-router.db]
    -h, --help       print help
    -V, --version    print version

ENV:
    KOU_ROUTER_BIND            listen address (overridden by --bind)
    KOU_ROUTER_DATABASE_URL    SQLite DSN (overridden by --db)
    KOU_ROUTER_ADMIN_PASSWORD  set/replace the web UI admin password without
                               the interactive first-run prompt (Docker)
    KOU_FRONTEND_DIST          serve the web UI from this directory instead
                               of the build embedded into the binary

On first run the admin password is asked interactively in the terminal;
in non-interactive environments set KOU_ROUTER_ADMIN_PASSWORD instead.
";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Serve,
    Daemon,
}

struct Cli {
    mode: Mode,
    bind: Option<String>,
    db: Option<String>,
}

fn parse_cli() -> Result<Cli, String> {
    let mut cli = Cli {
        mode: Mode::Serve,
        bind: None,
        db: None,
    };
    let mut args = env::args().skip(1);
    let mut first = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "serve" if first => cli.mode = Mode::Serve,
            "daemon" if first => cli.mode = Mode::Daemon,
            "--bind" => {
                cli.bind = Some(args.next().ok_or("--bind requires a value")?);
            }
            "--db" => {
                cli.db = Some(args.next().ok_or("--db requires a value")?);
            }
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("kou-router {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        first = false;
    }
    Ok(cli)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = match parse_cli() {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("error: {err}\n\n{HELP}");
            std::process::exit(2);
        }
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kou_router=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url = cli
        .db
        .or_else(|| env::var("KOU_ROUTER_DATABASE_URL").ok())
        .unwrap_or_else(|| "sqlite://kou-router.db".to_string());
    let default_bind = match cli.mode {
        Mode::Serve => "0.0.0.0:20128",
        Mode::Daemon => "127.0.0.1:20128",
    };
    let bind = cli
        .bind
        .or_else(|| env::var("KOU_ROUTER_BIND").ok())
        .unwrap_or_else(|| default_bind.to_string());

    let pool = init_db(&database_url).await?;
    let repository = Arc::new(SqliteRepository::new(pool));
    ensure_admin_password(&repository).await?;
    let state = AppState::new(repository);
    let app = match cli.mode {
        Mode::Serve => build_app(state),
        Mode::Daemon => build_headless_app(state),
    };

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let addr: SocketAddr = listener.local_addr()?;
    match cli.mode {
        Mode::Serve => tracing::info!(%addr, "kou-router listening (web UI + API)"),
        Mode::Daemon => tracing::info!(%addr, "kou-router daemon listening (headless API)"),
    }

    axum::serve(listener, app).await?;
    Ok(())
}

/// Гарантирует, что админ-пароль веб-UI задан до старта сервера:
/// `KOU_ROUTER_ADMIN_PASSWORD` всегда применяется как источник истины (Docker),
/// иначе на первом запуске пароль запрашивается интерактивно в терминале.
async fn ensure_admin_password(
    repository: &Arc<SqliteRepository>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(password) = env::var("KOU_ROUTER_ADMIN_PASSWORD") {
        bootstrap::set_admin_password(repository, &password)
            .await
            .map_err(|e| format!("KOU_ROUTER_ADMIN_PASSWORD: {e}"))?;
        tracing::info!(
            "admin password applied from KOU_ROUTER_ADMIN_PASSWORD, web UI auth enabled"
        );
        return Ok(());
    }

    if bootstrap::is_admin_configured(repository).await {
        return Ok(());
    }

    if !std::io::stdin().is_terminal() {
        return Err(
            "no admin password configured: run interactively to set one, \
                    or set KOU_ROUTER_ADMIN_PASSWORD"
                .into(),
        );
    }

    eprintln!(
        "First run: set the admin password for the web UI (min {} chars).",
        bootstrap::MIN_PASSWORD_LEN
    );
    loop {
        let password = rpassword::prompt_password("Admin password: ")?;
        if password.len() < bootstrap::MIN_PASSWORD_LEN {
            eprintln!(
                "Too short (min {} chars), try again.",
                bootstrap::MIN_PASSWORD_LEN
            );
            continue;
        }
        let confirm = rpassword::prompt_password("Confirm password: ")?;
        if password != confirm {
            eprintln!("Passwords don't match, try again.");
            continue;
        }
        bootstrap::set_admin_password(repository, &password).await?;
        eprintln!("Admin password set, web UI auth enabled.");
        return Ok(());
    }
}
