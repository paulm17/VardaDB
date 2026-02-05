# Advanced Features

## Polymorphism (Interfaces & Unions)

VardaDB supports complex data modeling using GraphQL Interfaces and Unions. This allows you to query across different types that share common fields.

### Interfaces
Define a common structure that other types implement.

```graphql
interface Character {
    id: ID
    name: String
    appearsIn: [Episode]
}

type Human implements Character {
    id: ID
    name: String
    appearsIn: [Episode]
    homePlanet: String
}

type Droid implements Character {
    id: ID
    name: String
    appearsIn: [Episode]
    primaryFunction: String
}
```

**Querying:**
Use inline fragments to retrieve type-specific data.

```graphql
query {
    queryCharacter {
        name
        ... on Human {
            homePlanet
        }
        ... on Droid {
            primaryFunction
        }
    }
}
```

### Unions
Unions allow a field to return one of several distinct Object types.

```graphql
union SearchResult = Human | Droid | Starship

type Query {
    search(text: String): [SearchResult]
}
```

## Cascading Deletes

The `@cascade` directive allows you to strictly enforce data cleanup. When a field is marked with `@cascade`, deleting the parent node will automatically delete the referenced child node(s).

**Warning**: This is a destructive operation that propagates down the graph.

```graphql
type Author {
    username: String
    posts: [Post] @cascade
}

type Post {
    title: String
    comments: [Comment] @cascade
}

type Comment {
    text: String
}
```

In this example:
1. Deleting an `Author` will automatically delete all their `Post`s.
2. Deleting those `Post`s will automatically delete all their `Comment`s.

This ensures no orphaned records remain in your database.

## Deep Mutations (Nested Writes)
As mentioned in the Edges section, VardaDB supports deep mutations. You can create a complex tree of data in a single transactional write.

```graphql
mutation {
    createAuthor(input: {
        username: "tolkien",
        posts: [
            { 
                title: "The Hobbit",
                comments: [
                    { text: "Great book!" }
                ]
            }
        ]
    })
}
```
All nodes (Author, Post, Comment) are created and linked atomically.
