use vardadb::realtime::bus::{EventBus, MutationEvent, MutationType, MutationSource};

#[tokio::test(flavor = "multi_thread")]
async fn test_bus_publish_subscribe() {
    let bus = EventBus::new();
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    let event = MutationEvent {
        type_name: "User".to_string(),
        uid: 123,
        mutation_type: MutationType::Update,
        source: MutationSource::Local,
        payload: None,
        metadata: None,
        timestamp: None,
    };

    bus.publish(event.clone());

    let received1 = rx1.recv().await.expect("rx1 failed");
    let received2 = rx2.recv().await.expect("rx2 failed");

    assert_eq!(received1.uid, 123);
    assert_eq!(received2.uid, 123);
    assert_eq!(received1.type_name, "User");
}
