use crate::bridge::fjall_resolver::FjallResolver;
use crate::ServerState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tracing::{error, info};

#[derive(Serialize, Deserialize, Debug)]
pub struct BulkRecord {
    pub type_name: String,
    pub uid: Option<u64>,
    pub fields: std::collections::HashMap<String, serde_json::Value>,
}

pub async fn start_tcp_listener(state: Arc<ServerState>, port: u16) {
    let addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind TCP Bulk Ingest listener on {}: {}", addr, e);
            return;
        }
    };

    info!("VardaDB TCP Bulk Ingest Stream open on {}", addr);

    loop {
        match listener.accept().await {
            Ok((mut socket, peer)) => {
                info!("Bulk ingest connection from {}", peer);
                let state_clone = state.clone();
                
                tokio::spawn(async move {
                    let mut batch_count = 0;
                    
                    // --- Database Name Handshake ---
                    // Read the first frame as a database name selector.
                    // If it parses as a BulkRecord array, treat it as data for the default db.
                    // Otherwise treat it as a UTF-8 database name string.
                    let mut len_bytes = [0u8; 4];
                    if let Err(e) = socket.read_exact(&mut len_bytes).await {
                        error!("Failed reading initial frame from {}: {}", peer, e);
                        return;
                    }
                    let payload_len = u32::from_le_bytes(len_bytes) as usize;
                    let mut buf = vec![0u8; payload_len];
                    if let Err(e) = socket.read_exact(&mut buf).await {
                        error!("Failed reading initial payload from {}: {}", peer, e);
                        return;
                    }
                    
                    // Try to parse as database name (simple string, not JSON array)
                    let (db_name, first_batch) = if buf.starts_with(b"[") || buf.starts_with(b"{") {
                        // It's JSON data, not a db name — use default
                        let default_db = state_clone.schemas.iter()
                            .find(|entry| entry.key() != "default")
                            .map(|entry| entry.key().clone())
                            .unwrap_or_else(|| "default".to_string());
                        (default_db, Some(buf))
                    } else {
                        // It's a database name
                        let name = String::from_utf8_lossy(&buf).trim().to_string();
                        info!("Bulk ingest: client selected database '{}'", name);
                        // Send ACK for db selection
                        use tokio::io::AsyncWriteExt;
                        if let Err(e) = socket.write_all(&[1u8]).await {
                            error!("Failed to send db selection ACK: {}", e);
                            return;
                        }
                        (if name.is_empty() { "default".to_string() } else { name }, None)
                    };
                    
                    // Create resolver targeting the correct database
                    let resolver = FjallResolver::with_db(
                        state_clone.storage.clone(), 
                        state_clone.event_bus.clone(),
                        db_name.clone()
                    );
                    
                    // Pre-fetch schema — try db-specific first, fall back to default
                    let schema_wrapper = state_clone.schemas.get(&db_name)
                        .or_else(|| state_clone.schemas.get("default"))
                        .expect("Missing schema");
                    let schema = schema_wrapper.read().await.clone();
                    
                    info!("Bulk ingest: using schema for db '{}' (types: {})", db_name, 
                          schema.type_metadata.len());

                    // Process first batch if the handshake frame was actually data
                    if let Some(first_buf) = first_batch {
                        if let Ok(records) = serde_json::from_slice::<Vec<BulkRecord>>(&first_buf) {
                            for record in records {
                                let (uniques, inverses, search_fields) = if let Some(meta) = schema.type_metadata.get(&record.type_name) {
                                    (&meta.uniques, &meta.inverses, &meta.search_fields)
                                } else {
                                    (&vec![], &vec![], &std::collections::HashMap::new())
                                };
                                let uid = record.uid.unwrap_or_else(|| {
                                   std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("Time").as_nanos() as u64
                                });
                                if let Err(e) = resolver.create_node_internal(&record.type_name, uid, record.fields, uniques, inverses, search_fields, crate::realtime::bus::MutationSource::Local, None) {
                                    error!("Bulk ingest resolver error for UID {}: {}", uid, e);
                                }
                            }
                            use tokio::io::AsyncWriteExt;
                            let _ = socket.write_all(&[1u8]).await;
                            batch_count += 1;
                        }
                    }

                    loop {
                        // 1. Read the length prefix (4 bytes)
                        let mut len_bytes = [0u8; 4];
                        if let Err(e) = socket.read_exact(&mut len_bytes).await {
                            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                                info!("Client {} disconnected cleanly. Inserted {} batches.", peer, batch_count);
                            } else {
                                error!("Failed reading frame length from {}: {}", peer, e);
                            }
                            break;
                        }

                        let payload_len = u32::from_le_bytes(len_bytes) as usize;
                        if payload_len > 1024 * 1024 * 50 { // 50 MB max batch size guard
                            error!("Payload too large from {}: {} bytes", peer, payload_len);
                            break;
                        }

                        // 2. Read the payload
                        let mut buf = vec![0u8; payload_len];
                        if let Err(e) = socket.read_exact(&mut buf).await {
                            error!("Failed reading payload from {}: {}", peer, e);
                            break;
                        }

                        // 3. Deserialize batch
                        let records: Vec<BulkRecord> = match serde_json::from_slice(&buf) {
                            Ok(r) => r,
                            Err(e) => {
                                error!("Failed to deserialize JSON bulk batch from {}: {}", peer, e);
                                break;
                            }
                        };

                        // 4. Stream into FjallResolver
                        for record in records {
                            let (uniques, inverses, search_fields) = if let Some(meta) = schema.type_metadata.get(&record.type_name) {
                                (&meta.uniques, &meta.inverses, &meta.search_fields)
                            } else {
                                (&vec![], &vec![], &std::collections::HashMap::new())
                            };
                            
                            let uid = record.uid.unwrap_or_else(|| {
                               let start = std::time::SystemTime::now();
                               let since_the_epoch = start.duration_since(std::time::UNIX_EPOCH).expect("Time went backwards");
                               since_the_epoch.as_nanos() as u64
                            });
                            
                            if let Err(e) = resolver.create_node_internal(
                                &record.type_name, 
                                uid, 
                                record.fields, 
                                uniques, 
                                inverses, 
                                search_fields, 
                                crate::realtime::bus::MutationSource::Local,
                                None
                            ) {
                                error!("Bulk ingest resolver error for UID {}: {}", uid, e);
                            }
                        }
                        
                        // Send ACK back to client to prevent TCP buffer overflow
                        use tokio::io::AsyncWriteExt;
                        if let Err(e) = socket.write_all(&[1u8]).await {
                            error!("Failed to send ACK to client: {}", e);
                            break;
                        }
                        
                        batch_count += 1;
                    }
                });
            }
            Err(e) => {
                error!("TCP Accept error: {}", e);
            }
        }
    }
}
