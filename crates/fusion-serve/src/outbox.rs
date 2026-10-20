//! Sending to one client without ever waiting for it.

use fusion_protocol::encode;
use fusion_protocol::v1::ServerMessage;
use fusion_viewserver::Session;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::TrySendError;

/// What happened to a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// Everything was queued.
    Sent,
    /// The queue was full: the session sends fresh snapshots once it drains.
    Behind,
    /// The client is gone.
    Gone,
}

/// One client's queue of encoded frames, bounded.
pub(crate) struct Outbox {
    tx: Sender<Vec<u8>>,
    behind: bool,
}

impl Outbox {
    pub(crate) fn new(tx: Sender<Vec<u8>>) -> Self {
        Self { tx, behind: false }
    }

    /// True if the client fell behind and its queue has not drained to half.
    pub(crate) fn waiting(&mut self) -> bool {
        if self.behind && self.tx.capacity() * 2 >= self.tx.max_capacity() {
            self.behind = false;
        }
        self.behind
    }

    /// Queues messages. If the queue fills, the rest are dropped, the
    /// session starts over with snapshots, and nothing more is sent until
    /// the queue has drained to half.
    pub(crate) fn deliver(
        &mut self,
        messages: &[ServerMessage],
        session: &mut Session,
    ) -> Delivery {
        for message in messages {
            match self.tx.try_send(encode(message)) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.behind = true;
                    session.reset();
                    return Delivery::Behind;
                }
                Err(TrySendError::Closed(_)) => return Delivery::Gone,
            }
        }
        Delivery::Sent
    }
}

#[cfg(test)]
mod tests {
    use fusion_protocol::v1::server_message;
    use tokio::sync::mpsc;

    use super::*;

    fn ack(request: u64) -> ServerMessage {
        fusion_viewserver::ack(request)
    }

    #[test]
    fn messages_go_out_until_the_queue_is_full() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut outbox = Outbox::new(tx);
        let mut session = Session::default();
        assert!(!outbox.waiting());
        let first: Vec<ServerMessage> = (1..=6).map(ack).collect();
        assert_eq!(outbox.deliver(&first, &mut session), Delivery::Sent);
        let more = [ack(7), ack(8), ack(9)];
        assert_eq!(outbox.deliver(&more, &mut session), Delivery::Behind);
        // Waiting until at least half (4 of 8) is free again.
        for free in 1..=3 {
            rx.try_recv().expect("a frame");
            assert!(outbox.waiting(), "{free} of 8 free");
        }
        rx.try_recv().expect("a frame");
        assert!(!outbox.waiting(), "4 of 8 free: half drained");
        let next: ServerMessage =
            fusion_protocol::decode(&rx.try_recv().expect("a frame")).expect("decodes");
        assert!(matches!(next.body, Some(server_message::Body::Ack(a)) if a.request == 5));
    }

    #[test]
    fn a_closed_queue_means_the_client_is_gone() {
        let (tx, rx) = mpsc::channel(4);
        drop(rx);
        let mut outbox = Outbox::new(tx);
        assert_eq!(
            outbox.deliver(&[ack(1)], &mut Session::default()),
            Delivery::Gone
        );
        assert_eq!(outbox.deliver(&[], &mut Session::default()), Delivery::Sent);
    }
}
