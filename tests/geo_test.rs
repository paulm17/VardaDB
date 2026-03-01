use async_graphql::Request;
use std::sync::Arc;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::storage::backend::Storage;
use serde_json::Value;

#[tokio::test(flavor = "multi_thread")]
async fn test_geo_support() {
    let schema = vardadb::engine::schema::Schema::load_from_sdl(
        "
        type Store {
            id: ID
            name: String
            location: GeoPoint @search(by: [geo])
            area: Polygon
        }
        "
    ).unwrap();

    // Setup Resolver
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Storage::new(tmp_dir.path(), None).unwrap());
    let resolver = SqliteResolver::new(storage.clone(), "default");
    let boxed_resolver: Box<dyn vardadb::engine::resolver::Resolver + Send + Sync> = Box::new(resolver.clone());

    // Create a Store with Point and Polygon
    let mutation = "
        mutation {
            createStore(input: {
                name: \"Central Park Store\",
                location: { latitude: 40.785091, longitude: -73.968285 },
                area: {
                    exterior: [
                        { latitude: 40.78, longitude: -73.97 },
                        { latitude: 40.78, longitude: -73.96 },
                        { latitude: 40.79, longitude: -73.96 },
                        { latitude: 40.79, longitude: -73.97 },
                        { latitude: 40.78, longitude: -73.97 }
                    ],
                    interiors: []
                }
            }) {
                uid
            }
        }
    ";
    
    // Inject Resolver
    let req = Request::new(mutation).data(boxed_resolver);
    let resp = schema.execute(req).await;
    assert!(resp.errors.is_empty(), "Mutation failed: {:?}", resp.errors);

    // For query, we need a new resolver instance or share the storage?
    // Storage is Arc, so we can reuse or recreate.
    // The previous resolver was consumed by box? Box<T> owns T.
    // We need another one for the query or clone it (SqliteResolver is Clone).
    let resolver_query = SqliteResolver::new(storage, "default");
    let boxed_resolver_query: Box<dyn vardadb::engine::resolver::Resolver + Send + Sync> = Box::new(resolver_query);

    // Query it back
    let query = "
        query {
            queryStore {
                name
                location { latitude longitude }
                area {
                    exterior { latitude longitude }
                }
            }
        }
    ";

    let req_query = Request::new(query).data(boxed_resolver_query);
    let resp = schema.execute(req_query).await;
    assert!(resp.errors.is_empty(), "Query failed: {:?}", resp.errors);
    
    let data = resp.data.into_json().unwrap();
    let stores = data.get("queryStore").unwrap().as_array().unwrap();
    assert_eq!(stores.len(), 1);
    
    let store = &stores[0];
    assert_eq!(store.get("name").unwrap(), "Central Park Store");
    
    let loc = store.get("location").unwrap();
    assert!((loc.get("latitude").unwrap().as_f64().unwrap() - 40.785091).abs() < 0.0001);
    
    let area = store.get("area").unwrap();
    let exterior = area.get("exterior").unwrap().as_array().unwrap();
    assert_eq!(exterior.len(), 5);
    
    let p1 = &exterior[0];
    assert!((p1.get("latitude").unwrap().as_f64().unwrap() - 40.78).abs() < 0.0001);
    
    // 4. Test "within" Filter (Find Point in Polygon)
    // We have a store (Central Park) at 40.785091, -73.968285
    // Let's define a polygon that contains it.
    let query_within = "
        query {
            queryStore(filter: {
                location: {
                    within: {
                        exterior: [
                            {latitude: 40.0, longitude: -74.0},
                            {latitude: 40.0, longitude: -73.0},
                            {latitude: 41.0, longitude: -73.0},
                            {latitude: 41.0, longitude: -74.0},
                            {latitude: 40.0, longitude: -74.0}
                        ],
                        interiors: []
                    }
                }
            }) {
                name
            }
        }
    ";
    let res_within_json = schema.execute_with_resolver(query_within, Box::new(resolver.clone())).await;
    let res_within: Value = serde_json::from_str(&res_within_json).unwrap();
    let stores_within = res_within["data"]["queryStore"].as_array().expect("Expected array for within query");
    assert_eq!(stores_within.len(), 1, "Store should be found within the polygon");
    
    // 5. Test "intersects" Filter (Find Polygon intersecting Polygon)
    // Central Park Store has an 'area' (Polygon)
    // Let's define a polygon that intersects it.
    let query_intersects = "
        query {
            queryStore(filter: {
                area: {
                    intersects: {
                        exterior: [
                            {latitude: 40.78, longitude: -73.97},
                            {latitude: 40.78, longitude: -73.96},
                            {latitude: 40.79, longitude: -73.96},
                            {latitude: 40.79, longitude: -73.97},
                            {latitude: 40.78, longitude: -73.97}
                        ],
                        interiors: []
                    }
                }
            }) {
                name
            }
        }
    ";
    let res_intersects_json = schema.execute_with_resolver(query_intersects, Box::new(resolver.clone())).await;
    let res_intersects: Value = serde_json::from_str(&res_intersects_json).unwrap();
    let stores_int = res_intersects["data"]["queryStore"].as_array().expect("Expected array for intersects query");
    assert_eq!(stores_int.len(), 1, "Store area should intersect query polygon");
}
