//! A real view server for the tests, on this machine.

use std::thread;

use fusion_serve::{MemoryLayouts, ServeConfig, serve};
use tokio::sync::mpsc;

/// A free local port: bound, then let go.
pub fn free_address() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
    listener.local_addr().expect("an address").to_string()
}

/// Runs a view server for BTCUSDT (instrument 1) on `address` on a thread
/// of its own, until the returned sender is dropped.
pub fn start_server(
    address: &str,
) -> (
    mpsc::Sender<fusion_domain::MarketEvent>,
    thread::JoinHandle<()>,
) {
    let (tx, rx) = mpsc::channel(16);
    let address = address.to_owned();
    let server = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind(&address)
                .await
                .expect("binds");
            let symbols = ["BTCUSDT".to_owned()];
            serve(
                listener,
                &symbols,
                rx,
                MemoryLayouts::default(),
                ServeConfig::default(),
            )
            .await;
        });
    });
    (tx, server)
}
