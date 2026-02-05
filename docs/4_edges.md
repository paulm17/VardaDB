# How to Create Edges (Relationships)

VardaDB is a graph database, meaning relationships (edges) are first-class citizens. You can link nodes together to model complex data structures.

## Define Relationships in Schema

You define relationships by referencing other Types in your fields.

```graphql
type Author {
    id: ID!
    name: String
    posts: [Post] @hasInverse(field: "author")
}

type Post {
    id: ID!
    title: String
    author: Author @hasInverse(field: "posts")
}
```

### @hasInverse
The `@hasInverse` directive creates a bi-directional edge.
- If you add a Post to an Author's `posts` list, VardaDB automatically sets the Post's `author` field to that Author.
- This ensures your graph remains consistent.

## Creating Relationships

You can create relationships in two ways: **Deep Creation** (creating new nodes nested inside) or **Linking** to existing nodes.

### 1. Deep Creation (Nested Create)
You can create a tree of data in a single mutation.

```graphql
mutation {
    createAuthor(input: {
        name: "J.K. Rowling",
        posts: [
            { title: "Harry Potter 1" },
            { title: "Harry Potter 2" }
        ]
    }) {
        uid
        posts { uid title }
    }
}
```
This creates the Author AND the two Posts, linking them together automatically.

### 2. Linking Existing Nodes
To link to a node that already exists, you pass its `uid` in the input object.

Assuming we have an existing Author with `uid: "1001"`.

```graphql
mutation {
    createPost(input: {
        title: "New Book",
        author: { uid: "1001" }
    }) {
        uid
    }
}
```

Or updating an existing Author to add an existing Post (`uid: "2002"`):

```graphql
mutation {
    updateAuthor(id: "1001", input: {
        posts: [{ uid: "2002" }]
    })
}
```

## Traversing Edges (Graph Query)

Once linked, you can traverse the graph arbitrarily deep in your queries.

```graphql
query {
    getAuthor(id: "1001") {
        name
        posts {
            title
            author {
                name 
            }
        }
    }
}
```
