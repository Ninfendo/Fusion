//! A window process's end of the pipe.

use std::io::{self, Write as _};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use fusion_hub::{Bus, FromHub, ToHub, encode};
use fusion_layout::WindowId;
use interprocess::local_socket::{SendHalf, Stream, prelude::*};

use crate::lines::read_lines;
use crate::socket_name;

/// A window process's line to the hub in the launcher.
///
/// The pipe stays open until the process ends: the reader thread holds
/// it, and a window process ends when its window closes.
pub struct PipeBus {
    writer: SendHalf,
    inbox: Receiver<FromHub>,
}

impl PipeBus {
    /// Connects to the hub at `name` and says hello as `window`.
    ///
    /// `wake` runs on the reader thread after each message, so the window
    /// can draw at once instead of on its next timer. When the hub goes
    /// away, the window gets [`FromHub::Quit`].
    pub fn connect(
        name: &str,
        window: WindowId,
        wake: impl Fn() + Send + 'static,
    ) -> io::Result<Self> {
        let (reader, writer) = Stream::connect(socket_name(name)?)?.split();
        let (sender, inbox) = mpsc::channel();
        thread::Builder::new()
            .name("fusion-hub-reader".to_owned())
            .spawn(move || {
                read_lines(reader, |message| {
                    let message = message.unwrap_or_else(|err| FromHub::Notice {
                        text: format!("A message from the hub could not be read: {err}"),
                    });
                    let open = sender.send(message).is_ok();
                    wake();
                    open
                });
                // Without the hub the window cannot work.
                let _closed = sender.send(FromHub::Quit);
                wake();
            })?;
        let mut bus = Self { writer, inbox };
        bus.send(ToHub::Hello { window });
        Ok(bus)
    }
}

impl Bus for PipeBus {
    fn send(&mut self, message: ToHub) {
        // A failed write means the hub is gone. The reader thread then sees
        // the end and tells the window to quit.
        if let Ok(line) = encode(&message) {
            let _gone = self.writer.write_all(line.as_bytes());
        }
    }

    fn poll(&mut self) -> Vec<FromHub> {
        self.inbox.try_iter().collect()
    }

    fn can_open_windows(&self) -> bool {
        true
    }
}
