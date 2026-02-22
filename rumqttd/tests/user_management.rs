use reqwest::Client;
use rumqttc::{AsyncClient, MqttOptions, QoS};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn test_user_management_and_auth() {
    let _ = tracing_subscriber::fmt::try_init();

    // Note: To test this effectively, we would need to spin up the broker with custom
    // configs in the test (which happens in broker.rs `start()`).
    // Given the architecture of rumqttd tests, we can write a test that creates a temp sqlite db,
    // starts the broker, and uses reqwest to talk to the API, then rumqttc to connect.
    
    let db_path = format!("sqlite://{}/test_rumqttd.db?mode=rwc", std::env::temp_dir().display());
    let _ = std::fs::remove_file(std::env::temp_dir().join("test_rumqttd.db"));
    
    let mut config = rumqttd::Config::default();
    config.router.max_connections = 100;
    config.database = Some(rumqttd::DatabaseSettings {
        url: db_path.to_string(),
    });
    config.api = Some(rumqttd::ApiSettings {
        listen: "127.0.0.1:8880".parse().unwrap(),
    });

    let mut connections = rumqttd::ConnectionSettings {
        connection_timeout_ms: 5000,
        max_payload_size: 2048,
        max_inflight_count: 100,
        auth: None,
        external_auth: None,
        dynamic_filters: true,
    };
    
    let mut v4_servers = std::collections::HashMap::new();
    v4_servers.insert("1".to_string(), rumqttd::ServerSettings {
        name: "test-v4".to_string(),
        listen: "127.0.0.1:1883".parse().unwrap(),
        tls: None,
        next_connection_delay_ms: 10,
        connections,
    });
    config.v4 = Some(v4_servers);
    
    // Spawn broker in a separate task
    let mut broker = rumqttd::Broker::new(config);
    tokio::spawn(async move {
        // Since Broker::start is not async, we use a simple thread here.
        std::thread::spawn(move || {
            broker.start().unwrap();
        });
    });

    // Wait for broker and API to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    let client = Client::new();

    // 1. Create a user
    let res = client
        .post("http://127.0.0.1:8880/users")
        .json(&json!({
            "username": "testuser",
            "password": "testpassword"
        }))
        .send()
        .await
        .expect("Failed to create user");
    
    assert_eq!(res.status(), 201);

    // 2. Connect to MQTT with VALID credentials
    let mut mqttoptions = MqttOptions::new("client1", "127.0.0.1", 1883);
    mqttoptions.set_credentials("testuser", "testpassword");
    let (async_client, mut eventloop) = AsyncClient::new(mqttoptions, 10);
    
    async_client.subscribe("test/topic", QoS::AtMostOnce).await.unwrap();
    
    // Check next event, shouldn't panic (meaning we connected)
    match tokio::time::timeout(Duration::from_secs(2), eventloop.poll()).await {
        Ok(Ok(_)) => println!("Successfully connected with valid credentials"),
        _ => panic!("Failed to connect with valid credentials"),
    }

    // 3. Connect to MQTT with INVALID credentials
    let mut bad_mqttoptions = MqttOptions::new("client2", "127.0.0.1", 1883);
    bad_mqttoptions.set_credentials("testuser", "wrongpassword");
    let (_, mut bad_eventloop) = AsyncClient::new(bad_mqttoptions, 10);
    
    // Check next event, should fail
    match tokio::time::timeout(Duration::from_secs(2), bad_eventloop.poll()).await {
        Ok(Err(_)) => println!("Successfully rejected invalid credentials"),
        _ => panic!("Broker did not reject invalid credentials"),
    }

    // 4. Delete the user
    let delete_res = client
        .delete("http://127.0.0.1:8880/users/testuser")
        .send()
        .await
        .expect("Failed to delete user");
    
    assert_eq!(delete_res.status(), 204);
}
