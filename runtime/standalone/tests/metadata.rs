mod common;

#[tokio::test(flavor = "multi_thread")]
async fn standalone_bootstraps_local_metadata_from_standalone_config() {
    if !common::supports_tcp_loopback() {
        return;
    }

    let process = common::StandaloneProcess::spawn().await;

    let client = reqwest::Client::new();
    let info = client
        .get(&process.admin_url)
        .send()
        .await
        .expect("admin info");
    assert!(info.status().is_success());

    let body = info.text().await.expect("info body");
    assert!(body.contains("\"metadata_bootstrap\":\"local-config\""));
    assert!(body.contains("\"metadata_node_name\":\"standalone-test\""));

    process.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_rejects_partition_count_config() {
    let output = common::run_standalone_expect_failure(
        r#"
        num-partitions = 7
        "#,
    )
    .await;

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf8");
    assert!(stderr.contains("num-partitions"));
}

#[tokio::test(flavor = "multi_thread")]
async fn standalone_rejects_legacy_metadata_client_config() {
    let output = common::run_standalone_expect_failure(
        r#"
        [metadata-client]
        type = "etcd"
        addresses = ["127.0.0.1:2379"]
        "#,
    )
    .await;

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf8");
    assert!(stderr.contains("metadata-client"));
}
