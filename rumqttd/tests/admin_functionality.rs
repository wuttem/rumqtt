use rumqttd::{Broker, Config};
use std::time::Duration;
use bytes::Bytes;

#[test]
fn test_admin_functionality() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        test_logic().await;
    });
}

async fn test_logic() {
    println!("Starting test setup...");
    // Init logging
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stdout)
        .try_init();
    
    // 1. Setup Broker
    let mut config = Config::default();
    config.router.max_connections = 10;
    config.router.max_outgoing_packet_count = 200;
    config.router.max_segment_size = 1024 * 1024;
    config.router.max_segment_count = 10;
    let broker = Broker::new(config);
    println!("Broker created.");
    
    // 2. Create Admin Link
    let admin_client_id = "admin";
    let mut admin_link = broker.admin_link(admin_client_id).unwrap();
    
    // 3. Connect a normal client
    let client_id = "client1";
    let (mut client_tx, _client_rx) = broker.link(client_id).unwrap();
    
    // Allow some time for connection processing
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 4. Controller list clients
    let controller = broker.controller();
    println!("Requesting clients...");
    let clients = controller.get_clients().await.unwrap();
    println!("Clients received: {:?}", clients);
    assert!(clients.iter().any(|c| c.client_id == "client1"));
    assert!(clients.iter().any(|c| c.client_id == "admin"));
    
    // 5. Client publishes a message
    let topic = "hello/world";
    let payload = b"msg";
    // We need to wait for subscription of admin to propagate?
    // Admin subscribes to "#" in admin_link() call.
    // It should be fast.
    
    println!("Publishing message...");
    client_tx.publish(topic, payload.to_vec()).unwrap();
    println!("Message published.");
    
    // 6. Admin should receive it
    // We use timeout to avoid hanging if test fails
    println!("Waiting for admin message...");
    let recv_result = tokio::time::timeout(Duration::from_secs(1), admin_link.recv()).await;
    
    match recv_result {
        Ok(Ok(Some((publish, client_info)))) => {
            assert_eq!(publish.topic, Bytes::from(topic));
            assert_eq!(publish.payload, Bytes::from(payload.as_slice()));
            assert_eq!(client_info.client_id, "client1");
            println!("Admin received message from {}", client_info.client_id);
        }
        Ok(Ok(None)) => panic!("Admin link disconnected unexpectedly"),
        Ok(Err(e)) => panic!("Admin link error: {}", e),
        Err(_) => panic!("Timed out waiting for message"),
    }
    
    // 7. Controller disconnect client
    println!("Disconnecting client...");
    controller.disconnect_client("client1").await.unwrap();
    println!("Client disconnected.");
    
    // Verify disconnect
    // Retry a few times as disconnect is async
    let mut disconnected = false;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Use timeout for get_clients to avoid hanging
        let clients_result = tokio::time::timeout(Duration::from_secs(1), controller.get_clients()).await;
        // Unwrap the timeout result and the inner result
        // if timeout or error, valid to continue
        let clients_after = match clients_result {
            Ok(Ok(c)) => c,
            _ => continue, 
        };
        
        if !clients_after.iter().any(|c| c.client_id == "client1") {
            disconnected = true;
            break;
        }
    }
    assert!(disconnected, "Client1 was not disconnected");
    assert!(true);
}
/*
async fn test_logic() {
    // ...
}
*/
