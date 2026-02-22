use flume::Sender;
use rumqttd::{Broker, ClientInfo, Config};
use std::thread;

/// A simple actor that represents our external authentication service
struct AuthActor {
    rx: flume::Receiver<AuthRequest>,
}

impl AuthActor {
    fn new(rx: flume::Receiver<AuthRequest>) -> Self {
        Self { rx }
    }

    /// Block and process incoming requests sequentially
    fn start(self) {
        for req in self.rx.into_iter() {
            // Here you can do DB lookups, call external APIs, etc.
            let valid = req.username == "admin" && req.password == "password";

            let res = if valid {
                // Return Ok with a custom ClientInfo if we want to override client_id and tenant!
                Ok(Some(ClientInfo {
                    client_id: format!("{}_override", req.client_id),
                    tenant: Some("custom_tenant".to_string()),
                }))
            } else {
                Err("Invalid credentials".to_string())
            };

            // Send response back
            let _ = req.reply_tx.send(res);
        }
    }
}

/// Information sent to the auth actor
struct AuthRequest {
    client_id: String,
    username: String,
    password: String,
    _common_name: String,
    _organization: String,
    ca_path: Option<String>,
    reply_tx: tokio::sync::oneshot::Sender<Result<Option<ClientInfo>, String>>,
}

fn main() {
    let builder = tracing_subscriber::fmt()
        .pretty()
        .with_line_number(false)
        .with_file(false)
        .with_thread_ids(false)
        .with_thread_names(false);

    builder
        .try_init()
        .expect("initialized subscriber succesfully");

    // Load configuration
    let config = config::Config::builder()
        .add_source(config::File::with_name("rumqttd.toml"))
        .build()
        .unwrap();
    let mut config: Config = config.try_deserialize().unwrap();

    // Spawn the AuthActor
    let (tx, rx) = flume::bounded(100);
    let actor = AuthActor::new(rx);
    thread::spawn(move || {
        actor.start();
    });

    // Provide the config for [v4.1] server
    let server = config.v4.as_mut().and_then(|v4| v4.get_mut("1")).unwrap();

    // Set the auth handler, which bridges rumqttd's tokio async context to our AuthActor
    server.set_auth_handler(
        move |client_id: String,
              username: String,
              password: String,
              common_name: String,
              organization: String,
              ca_path: Option<String>| {
            let tx: Sender<AuthRequest> = tx.clone();
            async move {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                let req = AuthRequest {
                    client_id,
                    username,
                    password,
                    _common_name: common_name,
                    _organization: organization,
                    ca_path,
                    reply_tx,
                };

                // Send request to actor
                if tx.send_async(req).await.is_err() {
                    return Err("Auth system unavailable".into());
                }

                // Wait for the result from the actor
                match reply_rx.await {
                    Ok(res) => res,
                    Err(_) => Err("Auth system dropped the request".into()),
                }
            }
        },
    );

    // Start the broker
    let mut broker = Broker::new(config);
    broker.start().unwrap();
}
