# How to Build a Schema

VardaDB uses a "Schema-First" approach. You define your data model using a string in the standard GraphQL Schema Definition Language (SDL). VardaDB parses this string and automatically configures the database, indexes, and API resolvers.

## Basic Type Definition

To define a data entity, use the `type` keyword. By default, VardaDB generates an `id` field for every type, but it's good practice to declare it explicitly if you want to expose it in your SDL (though VardaDB handles `ID` management internally).

```graphql
type User {
    username: String
    email: String
    age: Int
    isActive: Boolean
}
```

### Supported Scalar Types

VardaDB supports standard GraphQL scalars and some extended types:
- `ID`: Unique identifier (Internal u64).
- `String`: UTF-8 string.
- `Int`: Signed 32-bit integer.
- `Float`: Signed double-precision floating-point value.
- `Boolean`: `true` or `false`.
- `Int64`: Signed 64-bit integer.
- `DateTime`: ISO-8601 Date Time string.
- `GeoPoint`: Object with `latitude` and `longitude`.
- `Polygon`: Geometric polygon.
- `MultiPolygon`: Collection of polygons.

## Directives

Directives allow you to configure database behavior directly in your schema.

### @unique

Enforces a unique constraint on a field. The database will reject any creation or update that results in a duplicate value for this field.

```graphql
type User {
    email: String @unique
}
```

### @search

Enables indexing for a field, allowing you to filter and search on it.

```graphql
type Post {
    title: String @search(by: [term, fulltext])
    category: String @search(by: [term])
    publishedAt: DateTime @search
}
```

- **`term`**: Exact match indexing. Useful for filtering by precise values (e.g., categories, tags, IDs).
- **`fulltext`**: Full-text indexing with stemming and tokenization. Useful for search functionality (e.g., searching within titles or bodies).
- **Default**: If no arguments are provided (e.g., `@search`), it defaults to `term`.

### @vector

Marks a field to be used as a vector embedding placeholder. While the vector data is stored separately in the backend, this directive signals intent (and may be used by future tooling). Currently, VardaDB handles vectors primarily via the `search` query API rather than a direct field on the object, but referencing it here can be useful for clarity.

**(Note: Vector search is primarily accessed via the generated `search` query, not by querying this field directly as a scalar.)**

## Recursive Logic

VardaDB supports recursive types.
```graphql
type Category {
    name: String
    parent: Category
    children: [Category]
}
```

## Lists and Non-Null

- `[String]`: List of strings.
- `String!`: Non-nullable string (Required).
- `[String!]!`: Non-nullable list of non-nullable strings.

## Interfaces and Unions

VardaDB supports polymorphism via Interfaces and Unions.

```graphql
interface Animal {
    name: String
}

type Dog implements Animal {
    name: String
    breed: String
}

type Cat implements Animal {
    name: String
    livesLeft: Int
}
```

When querying, you can use fragments to retrieve specific fields:

```graphql
query {
    queryAnimal {
        name
        ... on Dog { breed }
        ... on Cat { livesLeft }
    }
}
```
