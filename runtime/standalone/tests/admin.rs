mod common;

#[tokio::test(flavor = "multi_thread")]
async fn standalone_admin_exposes_health_version_and_info() {
    if !common::supports_tcp_loopback() {
        return;
    }

    let process = common::StandaloneProcess::spawn().await;
    let client = reqwest::Client::new();

    let health = client
        .get(format!("{}/health", process.admin_url))
        .send()
        .await
        .expect("admin health");
    assert!(health.status().is_success());
    assert_eq!(
        health.text().await.expect("health body"),
        r#"{"status":"ready"}"#
    );

    let version = client
        .get(format!("{}/version", process.admin_url))
        .send()
        .await
        .expect("admin version");
    assert!(version.status().is_success());
    let version_body = version.text().await.expect("version body");
    assert!(version_body.contains("\"version\":\""));

    let info = client
        .get(&process.admin_url)
        .send()
        .await
        .expect("admin info");
    assert!(info.status().is_success());
    let info_body = info.text().await.expect("info body");
    assert!(info_body.contains("\"service\":\"restate-standalone-admin\""));
    assert!(info_body.contains("\"phase\":\"phase-5-standalone-runtime\""));
    assert!(info_body.contains("\"metadata_bootstrap\":\"local-config\""));
    assert!(info_body.contains("\"metadata_node_name\":\"standalone-test\""));
    assert!(info_body.contains("\"worker_runtime_started\":true"));
    assert!(info_body.contains("\"worker_runtime_recovered\":false"));
    assert!(info_body.contains("\"worker_sqlite_file\":\""));

    process.shutdown().await;
}
