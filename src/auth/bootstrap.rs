//! Admin password bootstrap shared by the web setup endpoint, the
//! interactive CLI prompt and the `KOU_ROUTER_ADMIN_PASSWORD` env path.

use crate::error::{AppError, AppResult};
use crate::repository::SqliteRepository;

pub const MIN_PASSWORD_LEN: usize = 8;

pub async fn is_admin_configured(repo: &SqliteRepository) -> bool {
    repo.get_setting_string("admin_password_hash").await.is_ok()
}

/// Hashes and stores the admin password (overwriting any previous one),
/// generates a JWT secret if missing and enables auth.
pub async fn set_admin_password(repo: &SqliteRepository, password: &str) -> AppResult<()> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(AppError::BadRequest(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    let hash = super::password::hash_password(password)?;
    repo.set_setting("admin_password_hash", &format!("\"{hash}\""))
        .await?;

    if repo.get_setting_string("jwt_secret").await.is_err() {
        let secret = super::jwt::generate_jwt_secret();
        repo.set_setting("jwt_secret", &format!("\"{secret}\""))
            .await?;
    }

    repo.set_setting("require_auth", "true").await?;
    Ok(())
}
