use rumqttd::{Broker, Config, ConnectionSettings, ServerSettings};
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use rumqttc::{AsyncClient, MqttOptions, QoS};
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    pretty_env_logger::init();

    // 1. Configure the broker with essential settings
    // Default config has 0 max_connections and no servers, so we must configure it manually.
    let mut config = Config::default();
    
    // Router configuration
    config.id = 0;
    config.router.max_connections = 10;
    config.router.max_outgoing_packet_count = 200;
    config.router.max_segment_size = 1024 * 1024; // 1MB
    config.router.max_segment_count = 10;
    
    // Server configuration (v4 MQTT on port 1883)
    let mut v4_config = HashMap::new();
    v4_config.insert(
        "demo".to_owned(),
        ServerSettings {
            name: "demo-server".to_owned(),
            listen: "127.0.0.1:1883".parse().unwrap(),
            tls: None,
            next_connection_delay_ms: 10,
            connections: ConnectionSettings {
                connection_timeout_ms: 100,
                max_payload_size: 2048,
                max_inflight_count: 100,
                auth: None,
                external_auth: None,
                dynamic_filters: true,
            },
        },
    );
    config.v4 = Some(v4_config);

    // console config to avoid panic or issues? defaulting to None is fine.

    let mut broker = Broker::new(config);

    // 2. Create the Admin Link (for monitoring)
    // The "admin" client ID is privileged in this context
    let mut admin_link = broker.admin_link("admin")?;

    // 3. Create the Broker Controller (for management)
    let controller = broker.controller();

    // 4. Start the broker
    // Broker::start() is blocking and runs the network servers. We spawn it in a separate thread.
    thread::spawn(move || {
        broker.start().unwrap();
    });

    // Spawn a task to print messages received by the admin link
    // Admin link automatically subscribes to "#" (all topics)
    tokio::spawn(async move {
        // AdminLink::recv() is async
        while let Ok(Some((publish, client_info))) = admin_link.recv().await {
            println!(
                "[ADMIN MONITOR] Received message on topic '{}' from client '{}': {:?}",
                std::str::from_utf8(&publish.topic).unwrap_or("INVALID_UTF8"),
                client_info.client_id,
                publish.payload
            );
        }
    });

    // 5. Simulate a client connecting and publishing
    let mut mqttoptions = MqttOptions::new("client-1", "127.0.0.1", 1883);
    mqttoptions.set_keep_alive(Duration::from_secs(5));

    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    
    // Spawn client loop
    tokio::spawn(async move {
        while let Ok(_event) = eventloop.poll().await {
            // Just running the loop to keep connection alive
        }
    });

    // Wait for client to connect
    tokio::time::sleep(Duration::from_secs(1)).await;

    // 6. Use Controller to list clients
    println!("\n[CONTROLLER] Listing connected clients:");
    match controller.get_clients().await {
        Ok(clients) => {
            for client in clients {
                println!(" - {}", client.client_id);
            }
        }
        Err(e) => eprintln!("Error getting clients: {:?}", e),
    }

    // 7. Client publishes a message
    println!("\n[CLIENT] Publishing message...");
    match client
        .publish("hello/rumqttd", QoS::AtLeastOnce, false, "Hello from client-1!")
        .await 
    {
        Ok(_) => println!("Message published"),
        Err(e) => eprintln!("Client publish error: {}", e),
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    // 8. Use Controller to disconnect the client
    println!("\n[CONTROLLER] Disconnecting 'client-1'...");
    match controller.disconnect_client("client-1").await {
        Ok(_) => println!("Successfully disconnected client-1"),
        Err(e) => eprintln!("Error disconnecting client: {:?}", e),
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    Ok(())
}
