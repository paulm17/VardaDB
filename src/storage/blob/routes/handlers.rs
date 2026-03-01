use axum::{
    body::Body,
    extract::{Path, State},
    http::{header::HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use async_graphql::Value;
use bytes::Bytes;
use std::sync::Arc;
use crate::storage::blob::{
    errors::VardaStorageError,
    file_info::FileInfo,
    routes::{headers::{check_header, parse_header}, BlobState, hashes::verify_chunk_checksum},
};

pub async fn file_status(
    State(state): State<Arc<BlobState>>,
    Path(id): Path<String>,
) -> Result<Response, VardaStorageError> {
    let info = state.info_storage.get_info(&id).await?;
    
    let mut builder = Response::builder()
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Offset", info.offset.to_string())
        .header("Cache-Control", "no-store");
        
    if let Some(length) = info.length {
        builder = builder.header("Upload-Length", length.to_string());
    } else {
        builder = builder.header("Upload-Defer-Length", "1");
    }

    if let Some(meta) = info.get_metadata_string() {
        builder = builder.header("Upload-Metadata", meta);
    }
    
    let response = builder.body(Body::empty())
        .map_err(|e| VardaStorageError::StorageError(e.to_string()))?;
        
    Ok(response)
}

pub async fn create_file(
    State(state): State<Arc<BlobState>>,
    headers: HeaderMap,
) -> Result<Response, VardaStorageError> {
    let length: Option<usize> = parse_header(&headers, "upload-length");
    let defer_size = check_header(&headers, "upload-defer-length", |v| v == "1");

    if length.is_none() && !defer_size {
        return Ok((StatusCode::BAD_REQUEST, "Upload-Length or Upload-Defer-Length required").into_response());
    }

    let meta = get_metadata(&headers);
    let file_id = uuid::Uuid::new_v4().to_string();
    
    let mut file_info = FileInfo::new(
        &file_id,
        length,
        None,
        "varda_data_storage".to_string(),
        meta,
    );

    let path = state.data_storage.create_file(&mut file_info).await?;
    file_info.path = Some(path);
    state.info_storage.set_info(&file_info, true).await?;

    let location = format!("/files/{}", file_id);
    let builder = Response::builder()
        .status(StatusCode::CREATED)
        .header("Location", location)
        .header("Tus-Resumable", "1.0.0");
        
    let response = builder.body(Body::empty())
        .map_err(|e| VardaStorageError::StorageError(e.to_string()))?;
        
    Ok(response)
}

pub async fn upload_chunk(
    State(state): State<Arc<BlobState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<Response, VardaStorageError> {
    let offset: Option<usize> = parse_header(&headers, "upload-offset");
    if offset.is_none() {
        return Ok((StatusCode::BAD_REQUEST, "Upload-Offset required").into_response());
    }
    
    let is_octet = check_header(&headers, "content-type", |v| v == "application/offset+octet-stream");
    if !is_octet {
        return Ok((StatusCode::UNSUPPORTED_MEDIA_TYPE, "Content-Type must be application/offset+octet-stream").into_response());
    }
    
    if let Some(checksum_header) = headers.get("upload-checksum").and_then(|h| h.to_str().ok()) {
        let cloned_bytes = bytes.clone();
        let header_clone = checksum_header.to_string();
        let is_valid = tokio::task::spawn_blocking(move || {
            verify_chunk_checksum(&header_clone, &cloned_bytes)
        }).await.unwrap_or(false);
        if !is_valid {
            return Ok((StatusCode::EXPECTATION_FAILED, "Checksum mismatch").into_response());
        }
    }

    // Grab lock for this upload
    let lock_arc = state.upload_locks.entry(id.clone()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone();
    let _guard = lock_arc.lock().await;

    let mut info = state.info_storage.get_info(&id).await?;
    
    if info.is_final || info.content_hash.is_some() {
        return Ok((StatusCode::FORBIDDEN, "Upload already final").into_response());
    }
    
    if offset.unwrap() != info.offset {
        return Ok((StatusCode::CONFLICT, "Offset mismatch").into_response());
    }
    
    // Defer-length update
    if let Some(new_len) = parse_header::<usize>(&headers, "upload-length") {
        if new_len < info.offset {
            return Err(VardaStorageError::WrongOffset);
        }
        if info.length.is_some() {
            return Ok((StatusCode::BAD_REQUEST, "Size already known").into_response());
        }
        info.deferred_size = false;
        info.length = Some(new_len);
    }
    
    if Some(info.offset) == info.length {
        return Ok((StatusCode::BAD_REQUEST, "Upload complete").into_response());
    }

    state.data_storage.add_bytes(&mut info, bytes).await?;
    
    let mut builder = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Offset", info.offset.to_string());
        
    // Check if finalized
    if info.length == Some(info.offset) {
        if let Some(path_str) = &info.path {
            let path = std::path::PathBuf::from(path_str);
            let (_final_path, hash) = state.data_storage.finalize(&info.id, &path).await?;
            info.content_hash = Some(hash.clone());
            info.is_final = true;
            
            let resolver = crate::bridge::sqlite_resolver::SqliteResolver::with_bus(
                state.server_state.storage.clone(),
                state.server_state.event_bus.clone(),
            );
            use crate::engine::resolver::Resolver;
            
            let mut fields = std::collections::HashMap::new();
            fields.insert("id".to_string(), Value::String(info.id.clone()));
            fields.insert("storageKey".to_string(), Value::String(info.id.clone()));
            let file_name = info.metadata.get("filename").unwrap_or(&info.id).clone();
            fields.insert("fileName".to_string(), Value::String(file_name));
            let mime_type = info.metadata.get("filetype").unwrap_or(&"application/octet-stream".to_string()).clone();
            fields.insert("mimeType".to_string(), Value::String(mime_type));
            fields.insert("size".to_string(), Value::Number(info.offset.into()));
            fields.insert("contentHash".to_string(), Value::String(hash.clone()));
            fields.insert("status".to_string(), Value::String("STAGED".to_string()));
            fields.insert("createdAt".to_string(), Value::String(chrono::Utc::now().to_rfc3339()));
            if let Some(meta) = serde_json::to_string(&info.metadata).ok() {
                fields.insert("metadata".to_string(), Value::String(meta));
            }

            let uniques = vec!["id".to_string()];
            let empty_inverses = vec![];
            
            let mut search_fields = std::collections::HashMap::new();
            search_fields.insert("storageKey".to_string(), vec!["exact".to_string()]);
            search_fields.insert("contentHash".to_string(), vec!["exact".to_string()]);
            search_fields.insert("status".to_string(), vec!["exact".to_string()]);

            let _ = resolver.create_node("FileRef", fields, &uniques, &empty_inverses, &search_fields, None);

            let mut q_fields = std::collections::HashMap::new();
            let q_id = uuid::Uuid::new_v4().to_string();
            q_fields.insert("id".to_string(), Value::String(q_id));
            q_fields.insert("fileRefId".to_string(), Value::String(info.id.clone()));
            q_fields.insert("status".to_string(), Value::String("PENDING".to_string()));
            q_fields.insert("retryCount".to_string(), Value::Number(0.into()));
            
            let mut q_search = std::collections::HashMap::new();
            q_search.insert("fileRefId".to_string(), vec!["exact".to_string()]);
            q_search.insert("status".to_string(), vec!["exact".to_string()]);

            let _ = resolver.create_node("UploadQueueEntry", q_fields, &vec!["id".to_string()], &empty_inverses, &q_search, None);

            // Return the URL hook.
            builder = builder.header("Varda-File-Url", format!("/files/hash/{}", info.content_hash.clone().unwrap()));
        }
    }

    state.info_storage.set_info(&info, false).await?;

    // Drop lock
    drop(_guard);
    
    // If complete, remove from lock map to prevent memory leak
    if info.is_final {
        state.upload_locks.remove(&id);
    }

    let response = builder.body(Body::empty())
        .map_err(|e| VardaStorageError::StorageError(e.to_string()))?;
        
    Ok(response)
}

pub async fn get_file(
    State(state): State<Arc<BlobState>>,
    Path(id): Path<String>,
) -> Result<Response, VardaStorageError> {
    // Basic streaming.
    // NOTE: production should check auth / FileRef logic first.
    let info = state.info_storage.get_info(&id).await?;
    state.data_storage.get_contents(&info).await
}

pub async fn get_blob_by_hash(
    State(state): State<Arc<BlobState>>,
    Path(hash): Path<String>,
) -> Result<Response, VardaStorageError> {
    let mut info = FileInfo::new("dummy", None, None, "varda_data_storage".to_string(), None);
    info.content_hash = Some(hash);
    state.data_storage.get_contents(&info).await
}

pub async fn delete_file(
    State(state): State<Arc<BlobState>>,
    Path(id): Path<String>,
) -> Result<Response, VardaStorageError> {
    if let Ok(info) = state.info_storage.get_info(&id).await {
        state.data_storage.remove_file(&info).await?;
        state.info_storage.remove_info(&id).await?;
    }
    
    let response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("Tus-Resumable", "1.0.0")
        .body(Body::empty())
        .unwrap();
        
    Ok(response)
}

fn get_metadata(headers: &HeaderMap) -> Option<std::collections::HashMap<String, String>> {
    use base64::{engine::general_purpose, Engine};
    
    headers
        .get("Upload-Metadata")
        .and_then(|h| h.to_str().ok())
        .map(|header_string| {
            let mut meta_map = std::collections::HashMap::new();
            for meta_pair in header_string.split(',') {
                let mut split = meta_pair.trim().split(' ');
                let key = split.next();
                let b64val = split.next();
                if key.is_none() || b64val.is_none() {
                    continue;
                }
                let value = general_purpose::STANDARD
                    .decode(b64val.unwrap())
                    .ok()
                    .and_then(|value| String::from_utf8(value).ok());
                if let Some(res) = value {
                    meta_map.insert(String::from(key.unwrap()), res);
                }
            }
            meta_map
        })
}
