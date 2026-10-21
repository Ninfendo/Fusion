//! PostgreSQL storage for Fusion.
//!
//! [`connect`] opens a pool of connections and brings the tables up to
//! date with the migrations in `migrations/`, which are built into the
//! program. [`PgLayouts`] keeps the window layouts in the `layouts` table.
//!
//! Queries are checked by the tests against a real database, not at
//! compile time, so building needs no database.

use std::future::Future;
use std::time::Duration;

use fusion_domain::LayoutStore;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// The migrations, built into the program.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!();

/// The most connections the pool opens.
const MAX_CONNECTIONS: u32 = 5;

/// How long to wait for a connection. A database that does not answer in
/// that time is an error, so a server does not hang at its start.
const CONNECT_WAIT: Duration = Duration::from_secs(10);

/// Connects to the database at `url` (for example
/// `postgres://fusion:fusion@127.0.0.1:55432/fusion`) and runs the
/// migrations it has not run yet.
pub async fn connect(url: &str) -> Result<PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(MAX_CONNECTIONS)
        .acquire_timeout(CONNECT_WAIT)
        .connect(url)
        .await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

/// Layouts in PostgreSQL.
#[derive(Debug, Clone)]
pub struct PgLayouts {
    pool: PgPool,
}

impl PgLayouts {
    /// Layouts in the database behind `pool`.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn failed(err: &sqlx::Error) -> String {
    format!("the layout store failed: {err}")
}

impl LayoutStore for PgLayouts {
    /// Stores the JSON as sent. Anything but a JSON object is refused.
    fn save(&self, name: String, json: String) -> impl Future<Output = Result<(), String>> + Send {
        let pool = self.pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO layouts (name, layout) VALUES ($1, $2) \
                 ON CONFLICT (name) DO UPDATE SET layout = EXCLUDED.layout, saved_at = now()",
            )
            .bind(name)
            .bind(json)
            .execute(&pool)
            .await
            .map(drop)
            .map_err(|err| failed(&err))
        }
    }

    fn load(&self, name: String) -> impl Future<Output = Result<Option<String>, String>> + Send {
        let pool = self.pool.clone();
        async move {
            sqlx::query_scalar("SELECT layout FROM layouts WHERE name = $1")
                .bind(name)
                .fetch_optional(&pool)
                .await
                .map_err(|err| failed(&err))
        }
    }
}
