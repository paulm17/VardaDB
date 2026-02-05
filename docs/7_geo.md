# Geo-Spatial Features

VardaDB provides native support for geographic data types and spatial querying, allowing you to build location-aware applications.

## Geo Types

Use these types in your schema to store spatial data.

### GeoPoint
Represents a specific coordinate on Earth. `latitude` and `longitude` are required floating-point values.

```graphql
type Restaurant {
    name: String
    location: GeoPoint
}
```

Format used in mutations:
```json
{
    "latitude": 40.7128,
    "longitude": -74.0060
}
```

### Polygon
Represents a closed shape defined by a series of points.
- **exterior**: The outer boundary of the polygon.
- **interiors**: (Optional) Lists of points defining "holes" inside the polygon.

```graphql
type Park {
    area: Polygon
}
```

### MultiPolygon
Represents a collection of multiple Polygons.

```graphql
type Country {
    borders: MultiPolygon
}
```

## Spatial Filtering

When querying, you can filter results based on spatial relationships. VardaDB generates specific input filters like `GeoPointFilter` for your types.

### Near Filter (Proximity Search)
Find items within a certain distance of a target point.

**Example: Find restaurants within 1000 meters:**

```graphql
query {
    queryRestaurant(filter: {
        location: {
            near: {
                coordinate: { latitude: 40.71, longitude: -74.00 },
                distance: 1000.0 
            }
        }
    }) {
        name
        location { latitude longitude }
    }
}
```

### Within Filter
Find items that are strictly *inside* a given Polygon.

**Example: Find parks inside a city boundary:**

```graphql
query {
    queryPark(filter: {
        area: {
            within: {
                exterior: [
                    { latitude: 40.0, longitude: -74.0 },
                    { latitude: 41.0, longitude: -74.0 },
                    { latitude: 41.0, longitude: -73.0 },
                    { latitude: 40.0, longitude: -74.0 } # Close loop
                ]
            }
        }
    }) { ... }
}
```

### Intersects Filter (Polygons)
Find items that overlap (intersect) with a given Polygon or MultiPolygon.

```graphql
query {
    queryCountry(filter: {
        borders: {
            intersects: {
                exterior: [ ... ]
            }
        }
    }) { ... }
}
```
