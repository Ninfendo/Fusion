//! The hub's messages over a local socket: a named pipe on Windows, a Unix
//! socket elsewhere. Desktop only; the browser uses BroadcastChannel.
//!
//! - [`PipeBus`] is a window process's [`Bus`](fusion_hub::Bus) to the hub
//!   in the launcher.
//! - [`HubServer`] is the launcher's end: one connection per window process.
//!
//! Each message is one line of JSON. A reader thread per connection turns
//! lines into messages, so no main loop ever blocks on a read.
//!
//! The socket accepts any process of the same user on the machine. Check
//! the peer's credentials before Fusion runs where other users can log in.

mod lines;
mod server;
mod window;

use std::io;

use interprocess::local_socket::{GenericNamespaced, Name, prelude::*};

pub use server::{HubServer, PipeEvent};
pub use window::PipeBus;

/// The socket name for one launcher run, from its process id, so two
/// launchers never share a socket.
pub fn hub_name(process: u32) -> String {
    format!("fusion-hub-{process}.sock")
}

fn socket_name(name: &str) -> io::Result<Name<'_>> {
    name.to_ns_name::<GenericNamespaced>()
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_launcher_has_its_own_socket() {
        assert_eq!(super::hub_name(42), "fusion-hub-42.sock");
        assert!(super::socket_name(&super::hub_name(42)).is_ok());
    }
}
