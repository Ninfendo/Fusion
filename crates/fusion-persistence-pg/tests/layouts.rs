//! Layouts in a real PostgreSQL.
//!
//! The tests need `FUSION_TEST_DATABASE_URL`, for example
//! `postgres://fusion:fusion@127.0.0.1:55432/fusion`. Without it they pass
//! without running, unless `FUSION_REQUIRE_DATABASE` is set, as CI does on
//! Linux.

use std::sync::atomic::{AtomicU32, Ordering};

use fusion_domain::LayoutStore;
use fusion_persistence_pg::{PgLayouts, connect};

fn database() -> Option<String> {
    match std::env::var("FUSION_TEST_DATABASE_URL") {
        Ok(url) => Some(url),
        Err(_) => {
            assert!(
                std::env::var_os("FUSION_REQUIRE_DATABASE").is_none(),
                "FUSION_REQUIRE_DATABASE is set, but FUSION_TEST_DATABASE_URL is not"
            );
            eprintln!("skipped: FUSION_TEST_DATABASE_URL is not set");
            None
        }
    }
}

/// A layout name no other test run uses, so runs can share a database.
fn fresh_name(what: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("test-{what}-{}-{nanos}-{n}", std::process::id())
}

async fn store() -> Option<PgLayouts> {
    let url = database()?;
    Some(PgLayouts::new(
        connect(&url).await.expect("connects and migrates"),
    ))
}

#[tokio::test]
async fn a_saved_layout_loads_as_sent_and_a_new_one_replaces_it() {
    let Some(layouts) = store().await else { return };
    let name = fresh_name("round-trip");
    assert_eq!(layouts.load(name.clone()).await, Ok(None));

    // Kept as sent, spacing and key order included.
    let first = r#"{"windows": [], "a":1}"#;
    layouts
        .save(name.clone(), first.into())
        .await
        .expect("saved");
    assert_eq!(layouts.load(name.clone()).await, Ok(Some(first.to_owned())));

    let second = r#"{"windows":[{"id":1}]}"#;
    layouts
        .save(name.clone(), second.into())
        .await
        .expect("replaced");
    assert_eq!(layouts.load(name).await, Ok(Some(second.to_owned())));
}

#[tokio::test]
async fn only_a_json_object_is_stored() {
    let Some(layouts) = store().await else { return };
    let name = fresh_name("refused");
    for bad in ["not json", "[1, 2]", "", "{\"open\": "] {
        let refused = layouts.save(name.clone(), bad.into()).await;
        assert!(
            refused
                .as_ref()
                .is_err_and(|e| e.starts_with("the layout store failed: ")),
            "{bad:?} gave {refused:?}"
        );
    }
    assert_eq!(layouts.load(name).await, Ok(None));
}

#[tokio::test]
async fn connecting_again_runs_no_migration_twice() {
    let Some(url) = database() else { return };
    connect(&url).await.expect("first");
    connect(&url).await.expect("second");
}

#[tokio::test]
async fn a_closed_pool_fails_with_a_message() {
    let Some(url) = database() else { return };
    let pool = connect(&url).await.expect("connects");
    let layouts = PgLayouts::new(pool.clone());
    pool.close().await;
    let name = fresh_name("closed");
    assert!(layouts.save(name.clone(), "{}".into()).await.is_err());
    assert!(layouts.load(name).await.is_err());
}

#[tokio::test]
async fn a_database_that_is_not_there_is_an_error() {
    // Not an address at all: refused before any connection is tried.
    let refused = connect("no address").await;
    assert!(refused.is_err());
}
