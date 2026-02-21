# Device State Manager Tutorial

This tutorial demonstrates how to embed the `rumqttd` broker inside a Rust application and use the newly added `AdminLink` to build a centralized processing hub. 

In many IoT systems, the broker isn't just a passive message router. You might want the server hosting the broker to actively listen for device updates, process them, and send state changes back to the devices. The `AdminLink` provides a simple, lock-free way to tap directly into the exact message stream without establishing a secondary network connection or configuring a secondary client.

## Scenario
We want to:
1. Start an embedded MQTT broker that listens for incoming device connections.
2. Intercept messages published by devices on the topic `device/<DEVICEID>/update`.
3. Process the update (simulated).
4. Send a response/acknowledgement directly back to the device on the topic `device/<DEVICEID>/state`.

## Implementation

The complete source code is available in `examples/device_state_manager.rs`. 

### 1. Embedded Broker Configuration
First, we must construct and configure the broker. By default, an empty `Config` sets `max_connections = 0` and provides no listeners. To accept network connections across `v4`, we must manually inject a `ServerSettings`.

```rust
let mut config = Config::default();

// Router configuration
config.id = 0;
config.router.max_connections = 10;
config.router.max_outgoing_packet_count = 200;
config.router.max_segment_size = 1048576; // 1MB
config.router.max_segment_count = 10;

// Listen on 127.0.0.1:1883
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
```

### 2. Creating the AdminLink
Before spawning the broker in the background, we request an `AdminLink`. The AdminLink establishes a specialized internal local connection, allowing us direct access to messages routing through the broker, using a fully non-blocking asynchronous receiver.

```rust
// The "admin" client ID is privileged in this context
let mut admin_link = broker.admin_link("admin")?;

// Start the broker in a separate thread. `broker.start()` is a blocking 
// operation that boots up all TCP listeners and background services.
thread::spawn(move || {
    broker.start().unwrap();
});
```

### 3. The Central Processor (Using AdminLink)
The AdminLink acts as a firehose, automatically creating a subscription to `#` internally so it receives every packet. 

We spawn an asynchronous task to poll `admin_link.recv()`. When a message arrives, we match the topic pattern against `device/<DEVICEID>/update`. If it matches, we parse the ID, process the data, and use `admin_link.publish(...)` to fire a response.

By utilizing `admin_link.publish`, we completely bypass the standard MQTT serialization/network transport overhead for our internal backend services.

```rust
tokio::spawn(async move {
    // AdminLink receives all messages routed by the broker natively.
    while let Ok(Some((publish, _client_info))) = admin_link.recv().await {
        
        let topic = std::str::from_utf8(&publish.topic).unwrap_or("");
        let parts: Vec<&str> = topic.split('/').collect();
        
        // Filter out for our specific topic structure
        if parts.len() == 3 && parts[0] == "device" && parts[2] == "update" {
            let device_id = parts[1];
            let payload = std::str::from_utf8(&publish.payload).unwrap_or("");
            
            // Generate a response!
            let response_payload = format!("State synchronized. Last update: {}", payload);
            let response_topic = format!("device/{}/state", device_id);

            // Publish the message directly into the internal Event Router!
            if let Err(e) = admin_link.publish(response_topic, response_payload.into()) {
                eprintln!("[ADMIN] Failed to publish state: {}", e);
            }
        }
    }
});
```

### 4. Demonstrating the Flow (Device Side)
In order to test the internal processor we just built, we simulate a standard MQTT Client (`client-sensor-123`). 

```rust
let mut mqttoptions = MqttOptions::new("client-sensor-123", "127.0.0.1", 1883);
let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

// DEVICE: Subscribe to our response channel
client.subscribe("device/client-sensor-123/state", QoS::AtLeastOnce).await?;

// Spin up the event loop in the background...
tokio::spawn(async move {
    while let Ok(event) = eventloop.poll().await {
        if let rumqttc::Event::Incoming(rumqttc::Packet::Publish(packet)) = event {
            let payload = std::str::from_utf8(&packet.payload).unwrap_or("INVALID");
            println!("\n[DEVICE] Received state update from admin! Payload: '{}'", payload);
        }
    }
});

// DEVICE: Send the update! 
client.publish(
    "device/client-sensor-123/update", 
    QoS::AtLeastOnce, 
    false, 
    "temp=22C, humidity=45%"
).await?;
```

By establishing processing systems via the `AdminLink`, IoT applications requiring complex synchronization logic can safely and optimally interact using Rust entirely on top of the native memory space without TCP roundtrips.

You can observe the outputs simply via:
```sh
cargo run --example device_state_manager
```
