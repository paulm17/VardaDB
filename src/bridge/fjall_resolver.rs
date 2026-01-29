use crate::engine::resolver::Resolver;
use crate::storage::backend::Storage;
use crate::storage::codec::Codec;
use async_graphql::Value;
use std::sync::Arc;
use byteorder::{BigEndian, ByteOrder};

use crate::realtime::bus::{EventBus, MutationEvent, MutationType};

#[derive(Clone)]
pub struct FjallResolver {
    pub storage: Arc<Storage>,
    pub bus: EventBus,
}

impl FjallResolver {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage, bus: EventBus::new() }
    }

    fn link_inverse(&self, target_uid: u64, inverse_field: &str, is_list: bool, self_uid: u64) -> Result<(), String> {
         let key = Codec::encode_data_key(target_uid, inverse_field);
         
         if is_list {
             let mut list = if let Ok(Some(bytes)) = self.storage.get(&key) {
                 serde_json::from_slice::<Vec<Value>>(&bytes).unwrap_or_default()
             } else {
                 Vec::new()
             };
             
             // Check if already exists
             let val_to_add = Value::String(self_uid.to_string());
             if !list.contains(&val_to_add) {
                  list.push(val_to_add);
                  let bytes = serde_json::to_vec(&list).map_err(|e| e.to_string())?;
                  self.storage.insert(&key, &bytes).map_err(|e| e.to_string())?;
             }
         } else {
             // 1:1 or N:1 - Overwrite
             let val = Value::String(self_uid.to_string());
             let bytes = serde_json::to_vec(&val).map_err(|e| e.to_string())?;
             self.storage.insert(&key, &bytes).map_err(|e| e.to_string())?;
         }
         Ok(())
    }

    fn unlink_inverse(&self, target_uid: u64, inverse_field: &str, is_list: bool, self_uid: u64) -> Result<(), String> {
         let key = Codec::encode_data_key(target_uid, inverse_field);
         
         if is_list {
             if let Ok(Some(bytes)) = self.storage.get(&key) {
                 if let Ok(mut list) = serde_json::from_slice::<Vec<Value>>(&bytes) {
                      let val_to_remove = Value::String(self_uid.to_string());
                      // Filter out
                      list.retain(|v| {
                          // Handle String vs Number comparison if needed, but strict equality for now
                          v != &val_to_remove && 
                          // Also try Number variant if we stored it as number?
                          // For safety, convert both to string for comparison?
                          match v {
                              Value::String(s) => s != &self_uid.to_string(),
                              Value::Number(n) => n.as_u64() != Some(self_uid),
                              _ => true
                          }
                      });
                      
                      let bytes = serde_json::to_vec(&list).map_err(|e| e.to_string())?;
                      self.storage.insert(&key, &bytes).map_err(|e| e.to_string())?;
                 }
             }
         } else {
             // 1:1 - If the current value IS self, remove it (delete key)
             if let Ok(Some(bytes)) = self.storage.get(&key) {
                 if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                      let matches = match val {
                          Value::String(s) => s == self_uid.to_string(),
                          Value::Number(n) => n.as_u64() == Some(self_uid),
                          _ => false
                      };
                      if matches {
                          self.storage.remove(&key).map_err(|e| e.to_string())?;
                      }
                 }
             }
         }
         Ok(())
    }

    fn write_term_index(&self, uid: u64, field: &str, text: &str, strategy: &str) -> Result<(), String> {
        let terms = crate::engine::tokenizer::Tokenizer::tokenize(text, strategy);
        for term in terms {
            // Key: [0x04][Pred][Startgy?][Term][UID]
            // We need to encode Strategy in the key now!
            // Or prefix field name? "name" -> "name.hash", "name.term"
            // Dgraph does <predicate> <token> <uid>. But it separates indices.
            // Let's use `Codec::encode_term_index_key(field, &term, uid)` but modify field to `field + "." + strategy`?
            // "name.exact", "name.hash".
            // This is clean.
            let index_field = if strategy == "term" { field.to_string() } else { format!("{}.{}", field, strategy) };
            
            let key = Codec::encode_term_index_key(&index_field, &term, uid);
            self.storage.insert(&key, &[]).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn remove_term_index(&self, uid: u64, field: &str, text: &str, strategy: &str) -> Result<(), String> {
        let tokens = crate::engine::tokenizer::Tokenizer::tokenize(text, strategy);
        for term in tokens {
             let index_field = if strategy == "term" { field.to_string() } else { format!("{}.{}", field, strategy) };
            let key = Codec::encode_term_index_key(&index_field, &term, uid);
            self.storage.remove(&key).map_err(|e| e.to_string())?;
        }
        Ok(())
    }


    fn check_condition(&self, stored_val: &Option<Value>, condition: &Value) -> bool {
        // If condition is a Map, it's a Filter Object (eq, gt, etc.)
        // If condition is a Scalar, it's an implicit Equality check (Backward Compat / scalar input)
        
        match condition {
            Value::Object(map) => {
                for (op, target) in map {
                    match op.as_str() {
                        "eq" => {
                             if let Some(sv) = stored_val {
                                 if sv != target { return false; }
                             } else {
                                 if target != &Value::Null { return false; }
                             }
                        }
                        "gt" => {
                            // Comparison Logic (only if types match or are compatible)
                            match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) { if !(sf > tf) { return false; } }
                                },
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    // Try parsing as i64 (Int64 parity)
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                        if !(si > ti) { return false; }
                                    } else if ss <= ts { return false; } // Lexical fallback
                                },
                                _ => {}
                            }
                        }
                        "lt" => {
                            match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) { if !(sf < tf) { return false; } }
                                },
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                        if !(si < ti) { return false; }
                                    } else if ss >= ts { return false; }
                                },
                                _ => {}
                            }
                        }
                        "ge" => {
                            match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) { if !(sf >= tf) { return false; } }
                                },
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                        if !(si >= ti) { return false; }
                                    } else if ss < ts { return false; }
                                },
                                _ => {}
                            }
                        }
                        "le" => {
                             match (stored_val, target) {
                                (Some(Value::Number(sn)), Value::Number(tn)) => {
                                    if let (Some(sf), Some(tf)) = (sn.as_f64(), tn.as_f64()) { if !(sf <= tf) { return false; } }
                                },
                                (Some(Value::String(ss)), Value::String(ts)) => {
                                    if let (Ok(si), Ok(ti)) = (ss.parse::<i64>(), ts.parse::<i64>()) {
                                        if !(si <= ti) { return false; }
                                    } else if ss > ts { return false; }
                                },
                                _ => {}
                            }
                        }
                        "contains" => {
                             if let (Some(Value::String(ss)), Value::String(ts)) = (stored_val, target) {
                                 if !ss.contains(ts) { return false; }
                             } else {
                                 return false; 
                             }
                        }
                        "between" => {
                             if let (Some(Value::Number(sn)), Value::List(items)) = (stored_val, target) {
                                 if items.len() == 2 {
                                     if let (Value::Number(min_v), Value::Number(max_v)) = (&items[0], &items[1]) {
                                         if let (Some(sf), Some(min_f), Some(max_f)) = (sn.as_f64(), min_v.as_f64(), max_v.as_f64()) {
                                             if sf < min_f || sf > max_f { return false; }
                                         }
                                     }
                                 }
                             }
                        }
                        "near" => {
                            // target is { "distance": Float, "coordinate": { "latitude": Float, "longitude": Float } }
                            if let Value::Object(near_args) = target {
                                if let (Some(Value::Number(dist_val)), Some(Value::Object(coord_map))) = (near_args.get("distance"), near_args.get("coordinate")) {
                                    if let (Some(Value::Number(lat_val)), Some(Value::Number(lon_val))) = (coord_map.get("latitude"), coord_map.get("longitude")) {
                                        if let (Some(max_meters), Some(target_lat), Some(target_lon)) = (dist_val.as_f64(), lat_val.as_f64(), lon_val.as_f64()) {
                                             // Check stored value
                                             // Stored: { "latitude": ..., "longitude": ... }
                                             if let Some(Value::Object(stored_map)) = stored_val {
                                                 if let (Some(Value::Number(s_lat_v)), Some(Value::Number(s_lon_v))) = (stored_map.get("latitude"), stored_map.get("longitude")) {
                                                     if let (Some(s_lat), Some(s_lon)) = (s_lat_v.as_f64(), s_lon_v.as_f64()) {
                                                         // Haversine Calculation
                                                         let earth_radius_m = 6371000.0;
                                                         let d_lat = (target_lat - s_lat).to_radians();
                                                         let d_lon = (target_lon - s_lon).to_radians();
                                                         let lat1 = s_lat.to_radians();
                                                         let lat2 = target_lat.to_radians();
                                                         
                                                         let a = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
                                                         let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
                                                         let distance = earth_radius_m * c;
                                                         
                                                         if distance > max_meters { return false; }
                                                     }
                                                 }
                                             }
                                        }
                                    }
                                }
                            }
                        }
                        "within" => {
                            // Check if stored Point is WITHIN target Polygon
                            if let Value::Object(polygon) = target {
                                if let Some(Value::Object(stored_map)) = stored_val {
                                     // Parse Stored Point
                                     if let (Some(Value::Number(lat_v)), Some(Value::Number(lon_v))) = (stored_map.get("latitude"), stored_map.get("longitude")) {
                                         if let (Some(lat), Some(lon)) = (lat_v.as_f64(), lon_v.as_f64()) {
                                              let point = geo::Point::new(lon, lat); // Geo uses (x, y) = (lon, lat)
                                              
                                              // Parse Target Polygon
                                              if let Some(Value::List(ext_list)) = polygon.get("exterior") {
                                                  let mut ext_coords = Vec::new();
                                                  for p in ext_list {
                                                      if let Value::Object(pmap) = p {
                                                          if let (Some(Value::Number(plat)), Some(Value::Number(plon))) = (pmap.get("latitude"), pmap.get("longitude")) {
                                                              if let (Some(ylat), Some(xlon)) = (plat.as_f64(), plon.as_f64()) {
                                                                  ext_coords.push((xlon, ylat));
                                                              }
                                                          }
                                                      }
                                                  }
                                                  if !ext_coords.is_empty() {
                                                       let line_string = geo::LineString::from(ext_coords);
                                                       let poly = geo::Polygon::new(line_string, vec![]); 
                                                       use geo::contains::Contains;
                                                       if !poly.contains(&point) { return false; }
                                                  }
                                              }
                                         }
                                     }
                                }
                            }
                        }
                        "intersects" => {
                             // Check if stored Polygon INTERSECTS target Polygon
                             if let Some(Value::Object(stored_map)) = stored_val {
                                 if let Some(Value::List(stored_ext)) = stored_map.get("exterior") {
                                      let mut stored_coords = Vec::new();
                                      for p in stored_ext {
                                           if let Value::Object(pmap) = p {
                                               if let (Some(Value::Number(plat)), Some(Value::Number(plon))) = (pmap.get("latitude"), pmap.get("longitude")) {
                                                   if let (Some(ylat), Some(xlon)) = (plat.as_f64(), plon.as_f64()) {
                                                       stored_coords.push((xlon, ylat));
                                                   }
                                               }
                                           }
                                      }
                                      if !stored_coords.is_empty() {
                                          let stored_poly = geo::Polygon::new(geo::LineString::from(stored_coords), vec![]);
                                          
                                          if let Value::Object(polygon) = target {
                                              if let Some(Value::List(ext_list)) = polygon.get("exterior") {
                                                  let mut target_coords = Vec::new();
                                                  for p in ext_list {
                                                      if let Value::Object(pmap) = p {
                                                          if let (Some(Value::Number(plat)), Some(Value::Number(plon))) = (pmap.get("latitude"), pmap.get("longitude")) {
                                                              if let (Some(ylat), Some(xlon)) = (plat.as_f64(), plon.as_f64()) {
                                                                  target_coords.push((xlon, ylat));
                                                              }
                                                          }
                                                      }
                                                  }
                                                  if !target_coords.is_empty() {
                                                      let target_poly = geo::Polygon::new(geo::LineString::from(target_coords), vec![]);
                                                      use geo::intersects::Intersects;
                                                      if !stored_poly.intersects(&target_poly) { return false; }
                                                  }
                                              }
                                          }
                                      }
                                 }
                             }
                        }
                        "in" => {
                            if let Value::List(list) = target {
                                if let Some(sv) = stored_val {
                                    if !list.contains(sv) { return false; }
                                } else {
                                     // If stored value is null, can it be IN list? Only if list has null.
                                     if !list.contains(&Value::Null) { return false; }
                                }
                            }
                        }
                        "ne" => {
                             if let Some(sv) = stored_val {
                                 if sv == target { return false; }
                             } else {
                                 if target == &Value::Null { return false; }
                             }
                        }
                        _ => {}
                    }
                }
                true
            }
            _ => {
                // Scalar Equality Fallback
                match stored_val {
                    Some(sv) => sv == condition,
                    None => condition == &Value::Null
                }
            }
        }
    }

    fn get_candidates(&self, type_name: &str, filter: &std::collections::HashMap<String, Value>) -> Option<std::collections::HashSet<u64>> {
        let mut candidates: Option<std::collections::HashSet<u64>> = None;

        for (field, condition) in filter {
            // 1. Check Unique Indexes (Exact Equality)
            // { email: { eq: "..." } } OR { email: "..." }
            let eq_value = match condition {
                Value::Object(map) => map.get("eq"),
                val => Some(val), // Scalar equality
            };

            if let Some(val) = eq_value {
                // How do we know if it's a unique field? 
                // The Resolver doesn't have the Schema metadata here. 
                // We rely on trying to look it up in the Unique Index.
                // Index Key: [0x03][Pred][Val]
                // Pred: Type.Field
                if let Ok(val_str) = serde_json::to_string(val) {
                    let index_pred = format!("{}.{}", type_name, field);
                    let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                    if let Ok(Some(bytes)) = self.storage.get(&idx_key) {
                        if bytes.len() == 8 {
                           let uid = BigEndian::read_u64(&bytes);
                           let mut set = std::collections::HashSet::new();
                           set.insert(uid);
                           
                           // Intersection
                           if let Some(current) = candidates {
                               candidates = Some(current.into_iter().filter(|u| set.contains(u)).collect());
                           } else {
                               candidates = Some(set);
                           }
                           continue; // Optimized this field
                        }
                    }
                }
            }

            // 2. Check Search Indexes
            if let Value::Object(map) = condition {
                 // Handle "allofterms"
                     if let Some(Value::String(terms_str)) = map.get("allofterms") {
                    let terms = crate::engine::tokenizer::Tokenizer::tokenize(terms_str, "term");
                    let mut field_uids = std::collections::HashSet::new();
                    let mut first_term = true;

                    for term in terms {
                        let prefix = Codec::encode_term_index_prefix(field, &term);
                        use std::ops::Bound;
                        let iter = self.storage.main_partition.range((Bound::Included(prefix.clone()), Bound::Unbounded));
                        
                        let mut term_uids = std::collections::HashSet::new();
                        for item in iter {
                            if let Ok((key, _)) = item {
                                if !key.starts_with(&prefix) { break; }
                                if key.len() >= 8 {
                                    let uid = BigEndian::read_u64(&key[key.len()-8..]);
                                    term_uids.insert(uid);
                                }
                            }
                        }

                        if first_term {
                            field_uids = term_uids;
                            first_term = false;
                        } else {
                            field_uids.retain(|u| term_uids.contains(u));
                        }
                    }
                    
                    if let Some(current) = candidates {
                        candidates = Some(current.into_iter().filter(|u| field_uids.contains(u)).collect());
                    } else {
                        candidates = Some(field_uids);
                    }
                }
                
                // Handle "anyofterms"
                if let Some(Value::String(terms_str)) = map.get("anyofterms") {
                     let terms = crate::engine::tokenizer::Tokenizer::tokenize(terms_str, "term");
                     let mut field_uids = std::collections::HashSet::new();
                     
                     for term in terms {
                        let prefix = Codec::encode_term_index_prefix(field, &term);
                        use std::ops::Bound;
                        let iter = self.storage.main_partition.range((Bound::Included(prefix.clone()), Bound::Unbounded));
                        
                        for item in iter {
                            if let Ok((key, _)) = item {
                                if !key.starts_with(&prefix) { break; }
                                if key.len() >= 8 {
                                    let uid = BigEndian::read_u64(&key[key.len()-8..]);
                                    field_uids.insert(uid);
                                }
                            }
                        }
                     }
                     if let Some(current) = candidates {
                        candidates = Some(current.into_iter().filter(|u| field_uids.contains(u)).collect());
                     } else {
                        candidates = Some(field_uids);
                     }
                }

                if let Some(Value::String(terms_str)) = map.get("alloftext") {
                    let terms = crate::engine::tokenizer::Tokenizer::tokenize(terms_str, "fulltext");
                    let index_field = format!("{}.fulltext", field);
                    let mut field_uids = std::collections::HashSet::new();
                    let mut first_term = true;

                    for term in terms {
                        let prefix = Codec::encode_term_index_prefix(&index_field, &term);
                        use std::ops::Bound;
                        let iter = self.storage.main_partition.range((Bound::Included(prefix.clone()), Bound::Unbounded));
                        
                        let mut term_uids = std::collections::HashSet::new();
                        for item in iter {
                            if let Ok((key, _)) = item {
                                if !key.starts_with(&prefix) { break; }
                                if key.len() >= 8 {
                                    let uid = BigEndian::read_u64(&key[key.len()-8..]);
                                    term_uids.insert(uid);
                                }
                            }
                        }

                        if first_term {
                            field_uids = term_uids;
                            first_term = false;
                        } else {
                            field_uids.retain(|u| term_uids.contains(u));
                        }
                    }
                    
                    if let Some(current) = candidates {
                        candidates = Some(current.into_iter().filter(|u| field_uids.contains(u)).collect());
                    } else {
                        candidates = Some(field_uids);
                    }
                }

                // Handle "anyoftext" (Stemmed)
                if let Some(Value::String(terms_str)) = map.get("anyoftext") {
                     let terms = crate::engine::tokenizer::Tokenizer::tokenize(terms_str, "fulltext");
                     let index_field = format!("{}.fulltext", field);
                     let mut field_uids = std::collections::HashSet::new();
                     
                     for term in terms {
                        let prefix = Codec::encode_term_index_prefix(&index_field, &term);
                        use std::ops::Bound;
                        let iter = self.storage.main_partition.range((Bound::Included(prefix.clone()), Bound::Unbounded));
                        
                        for item in iter {
                            if let Ok((key, _)) = item {
                                if !key.starts_with(&prefix) { break; }
                                if key.len() >= 8 {
                                    let uid = BigEndian::read_u64(&key[key.len()-8..]);
                                    field_uids.insert(uid);
                                }
                            }
                        }
                     }
                     if let Some(current) = candidates {
                        candidates = Some(current.into_iter().filter(|u| field_uids.contains(u)).collect());
                     } else {
                        candidates = Some(field_uids);
                     }
                }
            }
        }
        candidates
    }
    pub fn check_filter_recursive(&self, uid: u64, filter: &indexmap::IndexMap<async_graphql::Name, Value>) -> bool {
        for (key, condition) in filter {
            match key.as_str() {
                "and" => {
                    if let Value::List(list) = condition {
                        for sub in list {
                            if let Value::Object(map) = sub {
                                if !self.check_filter_recursive(uid, map) { return false; }
                            }
                        }
                    }
                }
                "or" => {
                    if let Value::List(list) = condition {
                         let mut any = false;
                         for sub in list {
                             if let Value::Object(map) = sub {
                                 if self.check_filter_recursive(uid, map) { any = true; break; }
                             }
                         }
                         if !any { return false; }
                    }
                }
                "not" => {
                    if let Value::Object(map) = condition {
                        if self.check_filter_recursive(uid, map) { return false; }
                    }
                }
                _ => {
                    // Regular Field
                     let d_key = Codec::encode_data_key(uid, key);
                     let stored_val = if let Ok(Some(bytes)) = self.storage.get(&d_key) {
                         serde_json::from_slice::<Value>(&bytes).ok()
                     } else { None };

                     if !self.check_condition(&stored_val, condition) { return false; }
                }
            }
        }
        true
    }
}

impl Resolver for FjallResolver {
    fn resolve(&self, uid: u64, field_name: &str) -> Option<Value> {
        if field_name == "id" {
            return Some(Value::String(uid.to_string()));
        }

        let key = Codec::encode_data_key(uid, field_name);
        match self.storage.get(&key) {
            Ok(Some(bytes)) => {
                serde_json::from_slice(&bytes).ok()
            }
            _ => None,
        }
    }

    fn find_uid(&self, index_name: &str, value: &str) -> Option<u64> {
        let key = Codec::encode_unique_index_key(index_name, value);
        match self.storage.get(&key) {
             Ok(Some(bytes)) if bytes.len() == 8 => {
                 Some(BigEndian::read_u64(&bytes))
             }
             _ => None,
        }
    }

    fn create_node(&self, type_name: &str, fields: std::collections::HashMap<String, Value>, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>) -> Result<u64, String> {
        // Simple UID generation: SystemTime nanos
        let start = std::time::SystemTime::now();
        let since_the_epoch = start
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards");
        let uid = since_the_epoch.as_nanos() as u64;

        for (field, value) in &fields {
             // Serialize Value to JSON bytes
            let val_bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
            
            // 2b. Write Term Index if needed
            if let Some(tokenizers) = search_fields.get(field) {
                if let Value::String(s) = value {
                     for strategy in tokenizers {
                         self.write_term_index(uid, field, s, strategy)?;
                     }
                }
            }

            // 1. Check Uniqueness if required
            if uniques.contains(&field) {
                 // Construct Index Index: Type.Field
                 let index_pred = format!("{}.{}", type_name, field);
                 let val_str = serde_json::to_string(&value).map_err(|e| e.to_string())?;
                 
                 let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                 
                 // Check existence
                 if let Ok(Some(_)) = self.storage.get(&idx_key) {
                     return Err(format!("Duplicate value for unique field: {}", field));
                 }
                 
                 // Write Index: Key -> UID
                 let mut uid_bytes = vec![0u8; 8];
                 BigEndian::write_u64(&mut uid_bytes, uid);
                 self.storage.insert(&idx_key, &uid_bytes).map_err(|e| e.to_string())?;
            }

            // 2. Write Data
            let key = Codec::encode_data_key(uid, &field);
            self.storage.insert(&key, &val_bytes).map_err(|e| e.to_string())?;
        }
        
        // 3. Write Type Index (for Listing/Scanning)
        let type_key_idx = Codec::encode_type_index_key(type_name, uid);
        self.storage.insert(&type_key_idx, &[]).map_err(|e| e.to_string())?;

        // 3b. Write Internal _type Predicate (for Polymorphism)
        let type_data_key = Codec::encode_data_key(uid, "_type");
        let type_val_bytes = serde_json::to_vec(&Value::String(type_name.to_string())).expect("Serialization failed");
        self.storage.insert(&type_data_key, &type_val_bytes).map_err(|e| e.to_string())?;

        // 3c. Emit Event
        self.bus.publish(MutationEvent {
            type_name: type_name.to_string(),
            uid,
            mutation_type: MutationType::Create,
        });

        // 4. Handle @hasInverse
        for info in inverses {
             if let Some(val) = fields.get(&info.field) {
                 // value could be Single UID or List of UIDs
                 let mut target_uids = Vec::new();
                 
                 match val {
                     Value::String(s) => {
                         if let Ok(id) = s.parse::<u64>() { target_uids.push(id); }
                     }
                     Value::Number(n) => {
                         if let Some(id) = n.as_u64() { target_uids.push(id); }
                     }
                     Value::List(items) => {
                         for item in items {
                             match item {
                                  Value::String(s) => { if let Ok(id) = s.parse::<u64>() { target_uids.push(id); } }
                                  Value::Number(n) => { if let Some(id) = n.as_u64() { target_uids.push(id); } }
                                  _ => {}
                             }
                         }
                     }
                     _ => {}
                 }

                 for target_uid in target_uids {
                     self.link_inverse(target_uid, &info.inverse_field, info.inverse_is_list, uid)?;
                 }
             }
        }

        Ok(uid)
    }










    fn scan_nodes(&self, type_name: &str, filter: std::collections::HashMap<String, Value>, sort: std::collections::HashMap<String, Value>, first: Option<usize>, after: Option<String>) -> Vec<u64> {
        // Optimization: Smallest Set First
        let candidate_set = self.get_candidates(type_name, &filter);
        // Removed candidate set logic, always perform full scan or scan from cursor.

        let mut filter_im = indexmap::IndexMap::new();
        for (k, v) in &filter {
            filter_im.insert(async_graphql::Name::new(k), v.clone());
        }

        let prefix = Codec::encode_type_prefix(type_name);
        let needs_sorting = !sort.is_empty();
        
        // If we have a candidate set, we iterate THAT instead of the DB prefix scan
        // UNLESS we need to sort, in which case we still might generally fetch all, but we can filter the candidate set.
        
        let mut uids = Vec::new();

        if let Some(ref candidates) = candidate_set {
            // We have a narrowed set.
            // Just iterate the candidates and verify other filters.
            
            // Try parallelize if set is large enough?
            // For now, always parallelize as user requested it explicitly.
            use rayon::prelude::*;
            
            // Collect into Vec for Rayon (HashSet is not parallel iterator by default usually, or needs explicit support)
            // Rayon supports HashSet parallel iter if we import it.
            // But strict order for vector collection?
            
            let mut matched_uids: Vec<u64> = candidates.par_iter()
                .filter(|uid| {
                     let matches_filter = if filter.is_empty() {
                         true
                     } else {
                         self.check_filter_recursive(**uid, &filter_im)
                     };
                     matches_filter
                })
                .cloned()
                .collect();
            
            uids.append(&mut matched_uids);

            // Candidate set has no order guarantees. We MUST sort if pagination/sorting is active.
            // If no explicit sort, we should probably sort by UID for consistency?
            // Existing logic for full scan yields sorted by key (UID).
            if !needs_sorting {
                 uids.sort(); 
                 // Handle pagination below
            }

        } else {
             // FULL SCAN FALLBACK
            let start_key = if !needs_sorting {
                 if let Some(cursor) = after.clone() {
                     let uid = cursor.parse::<u64>().unwrap_or(0);
                     if uid == u64::MAX { return vec![]; }
                     Codec::encode_type_index_key(type_name, uid + 1)
                 } else {
                     prefix.clone()
                 }
            } else {
                prefix.clone()
            };

            use std::ops::Bound;
            let iter = self.storage.main_partition.range((Bound::Included(start_key), Bound::Unbounded));

            for item in iter {
                 if let Ok((key, _)) = item {
                     if !key.starts_with(&prefix) { break; }
                     if key.len() >= 8 {
                         let uid = BigEndian::read_u64(&key[key.len()-8..]);
                        
                         let matches_filter = if filter.is_empty() {
                             true
                         } else {
                             self.check_filter_recursive(uid, &filter_im)
                         };

                         if matches_filter {
                             uids.push(uid);
                             // If NO sorting, we can break early
                             if !needs_sorting {
                                 if let Some(limit) = first {
                                     if uids.len() >= limit { break; }
                                 }
                             }
                         }
                     }
                 }
            }
        }

        if needs_sorting {
            // In-Memory Sort
            if let Some((field, direction)) = sort.iter().next() {
                let asc = match direction {
                    Value::String(s) => s == "ASC",
                     _ => true
                };
                
                uids.sort_by(|a, b| {
                    let val_a = self.resolve(*a, field);
                    let val_b = self.resolve(*b, field);
                    
                    let cmp = match (val_a, val_b) {
                        (Some(Value::Number(na)), Some(Value::Number(nb))) => {
                             na.as_f64().partial_cmp(&nb.as_f64()).unwrap_or(std::cmp::Ordering::Equal)
                        },
                        (Some(Value::String(sa)), Some(Value::String(sb))) => {
                            sa.cmp(&sb)
                        },
                         (None, Some(_)) => std::cmp::Ordering::Less,
                         (Some(_), None) => std::cmp::Ordering::Greater,
                         _ => std::cmp::Ordering::Equal
                    };
                    
                    if asc { cmp } else { cmp.reverse() }
                });
            }
        }
        
        // Apply Pagination (If sorted OR if Candidate Set was used [since we sorted it manually])
        // If Full Scan + No Sort, we already applied limit inside loop.
        // But logic is cleaner if we just apply it here if we haven't yet?
        // Full Scan + No Sort breaks early, so uids.len() <= limit.
        // But `after` logic?
        // Candidate Set + No Sort -> We sorted manually by UID. Need to apply `first` / `after`.
        
        let apply_pagination = needs_sorting || candidate_set.is_some();

        if apply_pagination {
            if let Some(cursor_uid_str) = after {
                 if let Ok(cursor_uid) = cursor_uid_str.parse::<u64>() {
                     if let Some(pos) = uids.iter().position(|u| *u == cursor_uid) {
                         uids = uids.into_iter().skip(pos + 1).collect();
                     }
                 }
            }
            if let Some(limit) = first {
                uids.truncate(limit);
            }
        }

        uids
    }

    fn update_node(&self, type_name: &str, uid: u64, fields: std::collections::HashMap<String, Value>, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> {
        // 0. Update Search Indexes (Get Old -> Remove -> Add New)
         for (field, value) in &fields {
             if let Some(tokenizers) = search_fields.get(field) {
                 let data_key = Codec::encode_data_key(uid, field);
                 if let Ok(Some(old_bytes)) = self.storage.get(&data_key) {
                     if let Ok(Value::String(s)) = serde_json::from_slice::<Value>(&old_bytes) {
                          for strategy in tokenizers {
                              self.remove_term_index(uid, field, &s, strategy)?;
                          }
                     }
                 }
                 if let Value::String(s) = value {
                      for strategy in tokenizers {
                          self.write_term_index(uid, field, s, strategy)?;
                      }
                 }
             }
         }

        // 1. Check Uniqueness Constraints First
        for (field, value) in &fields {
            if uniques.contains(field) {
                let index_pred = format!("{}.{}", type_name, field);
                let val_str = serde_json::to_string(value).map_err(|e| e.to_string())?;
                let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);

                match self.storage.get(&idx_key) {
                    Ok(Some(existing_uid_bytes)) => {
                        let existing_uid = BigEndian::read_u64(&existing_uid_bytes);
                        if existing_uid != uid {
                            return Err(format!("Duplicate value for unique field: {}", field));
                        }
                    }
                    _ => {}
                }
            }
        }
        
        // 2. Handle Inverses (Read-Verify-Link)
        for info in inverses {
             if let Some(new_val) = fields.get(&info.field) {
                  // A. UNLINK OLD
                  let data_key = Codec::encode_data_key(uid, &info.field);
                  if let Ok(Some(old_bytes)) = self.storage.get(&data_key) {
                      let mut old_targets = Vec::new();
                      if let Ok(old_val) = serde_json::from_slice::<Value>(&old_bytes) {
                          match old_val {
                               Value::String(s) => { if let Ok(id) = s.parse::<u64>() { old_targets.push(id); } }
                               Value::Number(n) => { if let Some(id) = n.as_u64() { old_targets.push(id); } }
                               Value::List(items) => {
                                   for item in items {
                                       match item {
                                            Value::String(s) => { if let Ok(id) = s.parse::<u64>() { old_targets.push(id); } }
                                            Value::Number(n) => { if let Some(id) = n.as_u64() { old_targets.push(id); } }
                                            _ => {}
                                       }
                                   }
                               }
                               _ => {}
                          }
                      }
                      for old_target in old_targets {
                          self.unlink_inverse(old_target, &info.inverse_field, info.inverse_is_list, uid)?;
                      }
                  }

                  // B. LINK NEW
                 let mut new_targets = Vec::new();
                 match new_val {
                     Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                     Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                     Value::List(items) => {
                         for item in items {
                             match item {
                                  Value::String(s) => { if let Ok(id) = s.parse::<u64>() { new_targets.push(id); } }
                                  Value::Number(n) => { if let Some(id) = n.as_u64() { new_targets.push(id); } }
                                  _ => {}
                             }
                         }
                     }
                     _ => {}
                 }

                 for new_target in new_targets {
                     self.link_inverse(new_target, &info.inverse_field, info.inverse_is_list, uid)?;
                 }
             }
        }

        // 3. Handle Unique Index Updates
        for (field, value) in &fields {
            if uniques.contains(field) {
                // Get OLD value
                let data_key = Codec::encode_data_key(uid, field);
                if let Ok(Some(old_val_bytes)) = self.storage.get(&data_key) {
                    if let Ok(old_val) = serde_json::from_slice::<Value>(&old_val_bytes) {
                        if &old_val != value {
                            // Removes OLD index
                            let old_val_str = serde_json::to_string(&old_val).unwrap_or_default();
                            let index_pred = format!("{}.{}", type_name, field);
                            let old_idx_key = Codec::encode_unique_index_key(&index_pred, &old_val_str);
                            self.storage.remove(&old_idx_key).map_err(|e| e.to_string())?;
                        }
                    }
                }
                
                // Add NEW index
                let index_pred = format!("{}.{}", type_name, field);
                let val_str = serde_json::to_string(value).map_err(|e| e.to_string())?;
                let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                
                let mut uid_bytes = vec![0u8; 8];
                BigEndian::write_u64(&mut uid_bytes, uid);
                self.storage.insert(&idx_key, &uid_bytes).map_err(|e| e.to_string())?;
            }
        }

        // 4. Write Data
        for (field, value) in fields {
            let key = Codec::encode_data_key(uid, &field);
            let val_bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
            self.storage.insert(&key, &val_bytes).map_err(|e| e.to_string())?;
        }

        self.bus.publish(MutationEvent {
            type_name: type_name.to_string(),
            uid,
            mutation_type: MutationType::Update,
        });

        Ok(())
    }

    fn delete_node(&self, type_name: &str, uid: u64, uniques: &[String], inverses: &[crate::engine::resolver::InverseInfo], search_fields: &std::collections::HashMap<String, Vec<String>>) -> Result<(), String> {
        // 0. Remove Search Indexes
        for (field, tokenizers) in search_fields {
            let data_key = Codec::encode_data_key(uid, field);
            if let Ok(Some(bytes)) = self.storage.get(&data_key) {
                if let Ok(Value::String(s)) = serde_json::from_slice::<Value>(&bytes) {
                     for strategy in tokenizers {
                         self.remove_term_index(uid, field, &s, strategy)?;
                     }
                }
            }
        }

        // 1. Handle Inverses (Unlink)
        // We must do this BEFORE deleting data, so we can see who we are linked to.
        for info in inverses {
             let data_key = Codec::encode_data_key(uid, &info.field);
             if let Ok(Some(bytes)) = self.storage.get(&data_key) {
                 let mut targets = Vec::new();
                 if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                      match val {
                           Value::String(s) => { if let Ok(id) = s.parse::<u64>() { targets.push(id); } }
                           Value::Number(n) => { if let Some(id) = n.as_u64() { targets.push(id); } }
                           Value::List(items) => {
                               for item in items {
                                   match item {
                                        Value::String(s) => { if let Ok(id) = s.parse::<u64>() { targets.push(id); } }
                                        Value::Number(n) => { if let Some(id) = n.as_u64() { targets.push(id); } }
                                        _ => {}
                                   }
                               }
                           }
                           _ => {}
                      }
                 }
                 
                 for target in targets {
                     self.unlink_inverse(target, &info.inverse_field, info.inverse_is_list, uid)?;
                 }
             }
        }

        // 2. Remove Unique Indexes
        for field in uniques {
            let data_key = Codec::encode_data_key(uid, field);
            if let Ok(Some(val_bytes)) = self.storage.get(&data_key) {
                if let Ok(val) = serde_json::from_slice::<Value>(&val_bytes) {
                     let val_str = serde_json::to_string(&val).unwrap_or_default();
                     let index_pred = format!("{}.{}", type_name, field);
                     let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                     self.storage.remove(&idx_key).map_err(|e| e.to_string())?;
                }
             }
        }

        // 3. Remove Type Index
        let type_key = Codec::encode_type_index_key(type_name, uid);
        self.storage.remove(&type_key).map_err(|e| e.to_string())?;

        // 4. Remove Data Keys (Scan Prefix)
        let prefix = Codec::encode_data_prefix(uid);
        use std::ops::Bound;
        let iter = self.storage.main_partition.range((Bound::Included(prefix.clone()), Bound::Unbounded));
        
        let mut keys_to_delete = Vec::new();
        for item in iter {
             if let Ok((key, _)) = item {
                 if !key.starts_with(&prefix) {
                     break;
                 }
                 keys_to_delete.push(key);
             }
        }

        for k in keys_to_delete {
            self.storage.remove(&k).map_err(|e| e.to_string())?;
        }

        self.bus.publish(MutationEvent {
             type_name: type_name.to_string(),
             uid,
             mutation_type: MutationType::Delete,
        });

        Ok(())
    }

    fn node_exists(&self, type_name: &str, uid: u64) -> bool {
        let type_key = Codec::encode_type_index_key(type_name, uid);
        self.storage.contains_key(&type_key).unwrap_or(false)
    }

    fn get_node_type(&self, uid: u64) -> Option<String> {
        let type_key = Codec::encode_data_key(uid, "_type");
        if let Ok(Some(bytes)) = self.storage.get(&type_key) {
            if let Ok(Value::String(s)) = serde_json::from_slice(&bytes) {
                return Some(s);
            }
        }
        None
    }

    fn subscribe_events(&self) -> EventBus {
        self.bus.clone()
    }
}
