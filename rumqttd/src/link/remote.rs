use crate::link::local::{LinkError, LinkRx, LinkTx};
use crate::link::network;
use crate::link::network::Network;
use crate::local::LinkBuilder;
use crate::protocol::{CertInfo, ConnAck, Connect, ConnectReturnCode, Login, Packet, Protocol};
use crate::router::{Event, Notification};
use crate::{ConnectionId, ConnectionSettings};

use flume::{RecvError, SendError, Sender, TrySendError};
use std::cmp::min;
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::time::error::Elapsed;
use tokio::{select, time};
use tracing::{trace, Span};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O")]
    Io(#[from] io::Error),
    #[error("Zero keep alive")]
    ZeroKeepAlive,
    #[error("Not connect packet")]
    NotConnectPacket(Packet),
    #[error("Network {0}")]
    Network(#[from] network::Error),
    #[error("Timeout")]
    Timeout(#[from] Elapsed),
    #[error("Channel send error")]
    Send(#[from] SendError<(ConnectionId, Event)>),
    #[error("Channel recv error")]
    Recv(#[from] RecvError),
    #[error("Got new session, disconnecting old one")]
    SessionEnd,
    #[error("Persistent session requires valid client id")]
    InvalidClientId,
    #[error("Unexpected router message")]
    NotConnectionAck,
    #[error("ConnAck error {0}")]
    ConnectionAck(String),
    #[error("Authentication error")]
    InvalidAuth,
    #[error("Channel try send error")]
    TrySend(#[from] TrySendError<(ConnectionId, Event)>),
    #[error("Link error = {0}")]
    Link(#[from] LinkError),
}

/// Orchestrates between Router and Network.
pub struct RemoteLink<P> {
    connect: Connect,
    pub(crate) connection_id: ConnectionId,
    network: Network<P>,
    link_tx: LinkTx,
    link_rx: LinkRx,
    notifications: VecDeque<Notification>,
    pub(crate) will_delay_interval: u32,
}

impl<P: Protocol> RemoteLink<P> {
    pub async fn new(
        router_tx: Sender<(ConnectionId, Event)>,
        cert_info: Option<CertInfo>,
        mut network: Network<P>,
        connect_packet: Packet,
        dynamic_filters: bool,
        assigned_client_id: Option<String>,
    ) -> Result<RemoteLink<P>, Error> {
        let Packet::Connect(connect, props, lastwill, lastwill_props, _) = connect_packet else {
            return Err(Error::NotConnectPacket(connect_packet));
        };

        // Register this connection with the router. Router replys with ack which if ok will
        // start the link. Router can sometimes reject the connection (ex max connection limit)
        let client_id = assigned_client_id.as_ref().unwrap_or(&connect.client_id);
        let clean_session = connect.clean_session;

        let topic_alias_max = props.as_ref().and_then(|p| p.topic_alias_max);
        let session_expiry = props
            .as_ref()
            .and_then(|p| p.session_expiry_interval)
            .unwrap_or(0);

        let delay_interval = lastwill_props
            .as_ref()
            .and_then(|f| f.delay_interval)
            .unwrap_or(0);

        // The Server delays publishing the Client’s Will Message until
        // the Will Delay Interval has passed or the Session ends, whichever happens first
        let will_delay_interval = min(session_expiry, delay_interval);

        let (link_tx, link_rx, notification) = LinkBuilder::new(client_id, router_tx)
            .cert_info(cert_info)
            .clean_session(clean_session)
            .last_will(lastwill)
            .last_will_properties(lastwill_props)
            .dynamic_filters(dynamic_filters)
            .topic_alias_max(topic_alias_max.unwrap_or(0))
            .build()?;

        let id = link_rx.id();
        Span::current().record("connection_id", id);

        if let Some(mut packet) = notification.into() {
            if let Packet::ConnAck(_ack, props) = &mut packet {
                let mut new_props = props.clone().unwrap_or_default();
                new_props.assigned_client_identifier = assigned_client_id;
                *props = Some(new_props);
                network.write(packet).await?;
            }
        }

        Ok(RemoteLink {
            connect,
            connection_id: id,
            network,
            link_tx,
            link_rx,
            notifications: VecDeque::with_capacity(100),
            will_delay_interval,
        })
    }

    pub async fn start(&mut self) -> Result<(), Error> {
        self.network.set_keepalive(self.connect.keep_alive);

        // Note:
        // Shouldn't result in bounded queue deadlocks because of blocking n/w send
        loop {
            select! {
                o = self.network.read() => {
                    let packet = o?;
                    let len = {
                        let mut buffer = self.link_tx.buffer();
                        buffer.push_back(packet);
                        self.network.readv(&mut buffer)?;
                        buffer.len()
                    };

                    trace!("Packets read from network, count = {}", len);
                    self.link_tx.notify().await?;
                }
                // Receive from router when previous when state isn't in collision
                // due to previously received data request
                o = self.link_rx.exchange(&mut self.notifications) => {
                    o?;
                    let mut packets = VecDeque::new();
                    let mut unscheduled = false;

                    for notif in self.notifications.drain(..) {
                        if let Some(packet) = notif.into() {
                            packets.push_back(packet);
                        } else {
                            unscheduled = true;
                        }

                    }
                    self.network.writev(packets).await?;
                    if unscheduled {
                        self.link_rx.wake().await?;
                    }
                }
            }
        }
    }
}

/// Read MQTT connect packet from network and verify it.
/// authentication and checks are done here.
pub async fn mqtt_connect<P>(
    config: Arc<ConnectionSettings>,
    network: &mut Network<P>,
    cert_info: &Option<CertInfo>,
) -> Result<(Packet, Option<crate::ClientInfo>), Error>
where
    P: Protocol,
{
    // Wait for MQTT connect packet and error out if it's not received in time to prevent
    // DOS attacks by filling total connections that the server can handle with idle open
    // connections which results in server rejecting new connections
    let connection_timeout_ms = config.connection_timeout_ms.into();
    let packet = time::timeout(Duration::from_millis(connection_timeout_ms), async {
        let packet = network.read().await?;
        Ok::<_, network::Error>(packet)
    })
    .await??;

    let (connect, _props, login) = match packet {
        Packet::Connect(ref connect, ref props, _, _, ref login) => (connect, props, login),
        packet => return Err(Error::NotConnectPacket(packet)),
    };

    Span::current().record("client_id", &connect.client_id);


    let client_info = handle_auth(config.clone(), login.as_ref(), &connect.client_id, cert_info.as_ref()).await?;

    // When keep_alive feature is disabled client can live forever, which is not good in
    // distributed broker context so currenlty we don't allow it.
    if connect.keep_alive == 0 {
        return Err(Error::ZeroKeepAlive);
    }

    let empty_client_id = connect.client_id.is_empty();
    let clean_session = connect.clean_session;

    if empty_client_id && !clean_session {
        let ack = ConnAck {
            session_present: false,
            code: ConnectReturnCode::ClientIdentifierNotValid,
        };

        let packet = Packet::ConnAck(ack, None);
        network.write(packet).await?;

        return Err(Error::InvalidClientId);
    }

    // Ok((connect, props, lastwill, lastwill_props))
    Ok((packet, client_info))
}

async fn handle_auth(
    config: Arc<ConnectionSettings>,
    login: Option<&Login>,
    client_id: &str,
    certinfo: Option<&CertInfo>,
) -> Result<Option<crate::ClientInfo>, Error> {
    if config.auth.is_none() && config.external_auth.is_none() {
        return Ok(None);
    }

    if client_id.starts_with("_broker") {
        return Err(Error::InvalidAuth);
    }

    //  If no login is provided, use empty string
    let (username, password) = match login {
        Some(login) => (&login.username, &login.password),
        None => (&"".to_string(), &"".to_string()),
    };

    // if we have certinfo, we can use it to authenticate
    let (organization, common_name, ca_path) = match certinfo {
        Some(certinfo) => {
            let org = certinfo.organization.to_owned().unwrap_or("".to_string());
            let cn = certinfo.common_name.to_owned();
            let ca = certinfo.ca_path.clone();
            (org, cn, ca)
        }
        None => ("".to_string(), "".to_string(), None),
    };

    if let Some(auth) = &config.external_auth {
        match auth(
            client_id.to_owned(),
            username.to_owned(),
            password.to_owned(),
            common_name.to_owned(),
            organization.to_owned(),
            ca_path,
        )
        .await
        {
            Ok(client_info) => return Ok(client_info),
            Err(_) => return Err(Error::InvalidAuth),
        }
    }

    if let Some(pairs) = &config.auth {
        if let Some(stored_password) = pairs.get(username) {
            if stored_password.as_bytes().ct_eq(password.as_bytes()).into() {
                return Ok(None);
            }
        }

        return Err(Error::InvalidAuth);
    }

    Err(Error::InvalidAuth)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use crate::{protocol::Login, ConnectionSettings};

    use super::handle_auth;

    fn config() -> ConnectionSettings {
        ConnectionSettings {
            connection_timeout_ms: 0,
            max_payload_size: 0,
            max_inflight_count: 0,
            auth: None,
            external_auth: None,
            dynamic_filters: false,
        }
    }

    fn login() -> Login {
        Login {
            username: "u".to_owned(),
            password: "p".to_owned(),
        }
    }

    #[tokio::test]
    async fn no_login_no_auth() {
        let cfg = Arc::new(config());
        let r = handle_auth(cfg, None, "", None).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn some_login_no_auth() {
        let cfg = Arc::new(config());
        let login = login();
        let r = handle_auth(cfg, Some(&login), "", None).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn login_matches_static_auth() {
        let login = login();
        let mut map = HashMap::<String, String>::new();
        map.insert(login.username.clone(), login.password.clone());

        let mut cfg = config();
        cfg.auth = Some(map);

        let r = handle_auth(Arc::new(cfg), Some(&login), "", None).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn login_fails_static_no_external() {
        let login = login();
        let mut map = HashMap::<String, String>::new();
        map.insert("wrong".to_owned(), "wrong".to_owned());

        let mut cfg = config();
        cfg.auth = Some(map);

        let r = handle_auth(Arc::new(cfg), Some(&login), "", None).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn login_fails_static_matches_external() {
        let login = login();

        let mut map = HashMap::<String, String>::new();
        map.insert("wrong".to_owned(), "wrong".to_owned());

        let dynamic = |_: String, _: String, _: String, _: String, _: String, _: Option<String>| async { Ok(None) };

        let mut cfg = config();
        cfg.auth = Some(map);
        cfg.set_auth_handler(dynamic);

        let r = handle_auth(Arc::new(cfg), Some(&login), "", None).await;
        assert!(r.is_ok());
    }

    #[tokio::test]
    async fn login_fails_static_fails_external() {
        let login = login();

        let mut map = HashMap::<String, String>::new();
        map.insert("wrong".to_owned(), "wrong".to_owned());

        let dynamic = |_: String, _: String, _: String, _: String, _: String, _: Option<String>| async { Err("".to_string()) };

        let mut cfg = config();
        cfg.auth = Some(map);
        cfg.set_auth_handler(dynamic);

        let r = handle_auth(Arc::new(cfg), Some(&login), "", None).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn external_auth_clousre_or_fnptr_type_check_or_fail_compile() {
        let closure = |_: String, _: String, _: String, _: String, _: String, _: Option<String>| async { Err("".to_string()) };
        async fn fnptr(_: String, _: String, _: String, _: String, _: String, _: Option<String>) -> Result<Option<crate::ClientInfo>, String> {
            Ok(None)
        }

        let mut cfg = config();
        cfg.set_auth_handler(closure);
        cfg.set_auth_handler(fnptr);
    }
}
