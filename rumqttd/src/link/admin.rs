use crate::link::local::{LinkError, LinkRx, LinkTx};
use crate::router::Notification;
use crate::protocol::Publish;
use crate::ClientInfo;
use bytes::Bytes;

pub struct AdminLink {
    pub(crate) tx: LinkTx,
    pub(crate) rx: LinkRx,
}

impl AdminLink {
    pub fn new(tx: LinkTx, rx: LinkRx) -> AdminLink {
        AdminLink { tx, rx }
    }

    pub fn publish<T: Into<Bytes>>(&mut self, topic: T, payload: T) -> Result<usize, LinkError> {
        self.tx.publish(topic, payload)
    }

    pub fn subscribe(&mut self, filter: &str) -> Result<usize, LinkError> {
        self.tx.subscribe(filter)
    }

    pub async fn recv(&mut self) -> Result<Option<(Publish, ClientInfo)>, LinkError> {
        loop {
            let notification = self.rx.next().await?;
            match notification {
                Some(Notification::Forward(forward)) => {
                    return Ok(Some((forward.publish, forward.client)));
                }
                Some(Notification::Disconnect(_, _)) => {
                    return Ok(None);
                }
                None => return Ok(None),
                _ => continue, // Ignore other notifications
            }
        }
    }
}
