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

    let mut broker = Broker::new(config);

    // 2. Create the Admin Link (for monitoring and responding)
    // The "admin" client ID is privileged
    let mut admin_link = broker.admin_link("admin")?;

    // 3. Start the broker in a separate thread
    // Broker::start() is blocking and runs the network servers. We spawn it in a separate thread.
    thread::spawn(move || {
        broker.start().unwrap();
    });

    // 4. Background task for handling Admin messages
    tokio::spawn(async move {
        // AdminLink will receive ALL messages flowing through the broker.
        // We will filter incoming messages and respond appropriately.
        while let Ok(Some((publish, _client_info))) = admin_link.recv().await {
            let topic = std::str::from_utf8(&publish.topic).unwrap_or("");
            
            // We want to process requests coming to: device/<DEVICEID>/update
            let parts: Vec<&str> = topic.split('/').collect();
            
            if parts.len() == 3 && parts[0] == "device" && parts[2] == "update" {
                let device_id = parts[1];
                let payload = std::str::from_utf8(&publish.payload).unwrap_or("");
                
                println!("[ADMIN] Processing update for {}: {}", device_id, payload);

                // Simulate processing the update logic...
                let response_payload = format!("State synchronized. Last update: {}", payload);
                
                // Formulate the response topic: device/<DEVICEID>/state
                let response_topic = format!("device/{}/state", device_id);

                println!("[ADMIN] Responding on: {}", response_topic);
                
                // Publish the response message back down to the device via the admin link
                if let Err(e) = admin_link.publish(response_topic, response_payload.into()) {
                    eprintln!("[ADMIN] Failed to publish state: {}", e);
                }
            }
        }
    });

    // 5. Simulate a device client connecting and sending an update
    let mut mqttoptions = MqttOptions::new("client-sensor-123", "127.0.0.1", 1883);
    mqttoptions.set_keep_alive(Duration::from_secs(5));

    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    
    // The device subscribes to its own state topic to receive responses
    client.subscribe("device/client-sensor-123/state", QoS::AtLeastOnce).await?;

    // Spawn the client event loop task so we can receive the response
    tokio::spawn(async move {
        while let Ok(event) = eventloop.poll().await {
            if let rumqttc::Event::Incoming(rumqttc::Packet::Publish(packet)) = event {
                let payload = std::str::from_utf8(&packet.payload).unwrap_or("INVALID");
                println!("\n[DEVICE] Received state update from admin! Payload: '{}'", payload);
            }
        }
    });

    // Wait for client to connect
    tokio::time::sleep(Duration::from_secs(1)).await;

    // 6. Device publishes an update
    println!("\n[DEVICE] Sending sensor update...");
    client
        .publish("device/client-sensor-123/update", QoS::AtLeastOnce, false, "temp=22C, humidity=45%")
        .await?;

    // Let the event loop run briefly to receive the response
    tokio::time::sleep(Duration::from_secs(2)).await;

    Ok(())
}
