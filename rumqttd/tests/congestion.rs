use rumqttd::{Broker, Config};
use std::time::Duration;

#[test]
fn test_congestion_auto_disconnect() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        // Init logging
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_writer(std::io::stdout)
            .try_init();

        // 1. Setup Broker
        let mut config = Config::default();
        config.router.max_connections = 10;
        config.router.max_outgoing_packet_count = 200;
        config.router.max_segment_size = 1024 * 1024;
        config.router.max_segment_count = 10;
        let broker = Broker::new(config);
        
        // 2. Create Admin Link with small channel capacity
        let admin_client_id = "admin";
        let mut _admin_link = broker.admin_link(admin_client_id, 5).unwrap();
        // admin_link will subscribe to "#" by default and buffer messages. 
        // We WON'T read from it, causing it to congest!

        // 3. Connect a "safe" client 
        let safe_id = "safe_client";
        let (mut safe_tx, _safe_rx) = broker.link(safe_id).unwrap();
        
        let controller = broker.controller();

        // Add 100 msgs/sec lower rate limit for the safe client
        controller.set_rate_limits(safe_id, Some(100.0), None).await.unwrap();

        // 4. Connect a "spam" client 
        let spam_id = "spam_client";
        let (mut spam_tx, mut spam_rx) = broker.link(spam_id).unwrap();

        // Add 5 msgs/sec lower rate limit for the spam client
        controller.set_rate_limits(spam_id, Some(5.0), None).await.unwrap();

        // Let the setup settle
        tokio::time::sleep(Duration::from_millis(100)).await;

        let topic = "hello/world";
        let payload = vec![0; 1024];

        // 5. Have safe_client publish 10 messages (under limit of 100/sec)
        for _ in 0..10 {
            safe_tx.try_publish(topic, payload.clone()).unwrap();
        }

        tokio::time::sleep(Duration::from_millis(50)).await;

        // 6. Have spam_client publish 20 messages (over limit of 5/sec bursts)
        for _ in 0..20 {
            // Note: try_publish might fail if the router drops the connection
            let _ = spam_tx.try_publish(topic, payload.clone());
        }

        // Wait a bit for router to detect congestion and disconnect
        tokio::time::sleep(Duration::from_millis(200)).await;

        let clients = controller.get_clients().await.unwrap();
        let safe_exists = clients.iter().any(|c| c.client_id == safe_id);
        let spam_exists = clients.iter().any(|c| c.client_id == spam_id);

        assert!(safe_exists, "Safe client was incorrectly disconnected!");
        assert!(!spam_exists, "Spam client was NOT disconnected during congestion!");
    });
}
