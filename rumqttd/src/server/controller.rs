use crate::router::Event;
use crate::{ClientInfo, ConnectionId};
use flume::Sender;

#[derive(Debug, Clone)]
pub struct BrokerController {
    router_tx: Sender<(ConnectionId, Event)>,
}

impl BrokerController {
    pub fn new(router_tx: Sender<(ConnectionId, Event)>) -> BrokerController {
        BrokerController { router_tx }
    }

    pub async fn get_clients(&self) -> Result<Vec<ClientInfo>, Error> {
        let (tx, rx) = flume::bounded(1);
        let event = Event::GetClients(tx);
        // We use 0 as connection ID for control events, similar to how LinkBuilder uses 0 for Connect
        self.router_tx.send_async((0, event)).await.map_err(Error::Send)?;
        let clients = rx.recv_async().await.map_err(Error::Recv)?;
        Ok(clients)
    }

    pub async fn disconnect_client(&self, client_id: &str) -> Result<(), Error> {
        let event = Event::ForceDisconnect(client_id.to_owned());
        self.router_tx.send_async((0, event)).await.map_err(Error::Send)?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Channel send error")]
    Send(#[from] flume::SendError<(ConnectionId, Event)>),
    #[error("Channel recv error")]
    Recv(#[from] flume::RecvError),
}
