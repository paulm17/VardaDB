# How to CRUD (Create, Read, Update, Delete)

Once your schema is loaded, VardaDB automatically generates a complete set of operations for each type.

Assuming a schema:
```graphql
type User {
    username: String @unique
    age: Int
    isActive: Boolean
}
```

## Create

Use the `create<Type>` mutation. It accepts an `input` object matching your type's fields.

```graphql
mutation {
    createUser(input: {
        username: "alice",
        age: 30,
        isActive: true
    }) {
        uid   # The generated internal ID
        username
    }
}
```

**Response:**
```json
{
    "data": {
        "createUser": {
            "uid": "1001",
            "username": "alice"
        }
    }
}
```

## Read (Get by ID)

To fetch a single item by its ID, use the `get<Type>` query.

```graphql
query {
    getUser(uid: "1001") {
        username
        age
    }
}
```

You can also fetch by unique fields if you defined `@unique` in your schema:

```graphql
query {
    getUser(username: "alice") {
        uid
        age
    }
}
```

## Read (Query List with Filters)

To fetch a list of items, use the `query<Type>` query. This query supports powerful filtering, sorting, and pagination.

```graphql
query {
    queryUser(
        filter: { age: { ge: 18 } }, 
        sort: { username: ASC },
        first: 10
    ) {
        username
        age
    }
}
```

### Filtering options
VardaDB supports extensive filter operators based on the field type.

**Common Operators:**
- `eq`: Equal to
- `in`: In a given list of values

**Numeric / DateTime Operators:**
- `lt`, `le`: Less than, Less than or equal
- `gt`, `ge`: Greater than, Greater than or equal
- `between`: Range `[min, max]`

**String Operators:**
- `contains`: Substring match
- `allofterms`: Contains all terms (exact terms)
- `anyofterms`: Contains any of the terms
- `alloftext`: Full-text match (stemmed)
- `anyoftext`: Full-text match any (stemmed)
- `lt`, `le`, `gt`, `ge`: Lexicographical comparison

**Boolean Operators:**
- `eq`: Equal to (true/false)

**Logical Operators (Combine filters):**
- `and`: List of filters, all must be true
- `or`: List of filters, at least one must be true
- `not`: Negate a filter

Example complex filter:
```graphql
query {
    queryUser(filter: {
        and: [
            { age: { gt: 20 } },
            { isActive: { eq: true } }
        ]
    }) { ... }
}
```

## Update

Use the `update<Type>` mutation. You must provide the `id` of the node to update. The `input` object allows for partial updates (only fields present in the input will be changed).

```graphql
mutation {
    updateUser(uid: "1001", input: {
        age: 31
    })
}
```
*Returns `true` if successful.*

## Delete

Use the `delete<Type>` mutation with the node `id`.

```graphql
mutation {
    deleteUser(uid: "1001")
}
```
*Returns `true` if successful.*

### Cascading Deletes
If your schema uses the `@cascade` directive on relationships, deleting a node will also recursively delete the linked nodes. Use with caution!
