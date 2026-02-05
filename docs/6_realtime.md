# Realtime Subscriptions

VardaDB supports real-time data synchronization through GraphQL Subscriptions. This allows clients to listen for changes (Creates, Updates, Deletes) happening in the database instantly.

## The `MutationEvent` System

Under the hood, every write operation in VardaDB publishes a `MutationEvent` to an internal event bus. The GraphQL Subscription API exposes these events.

### Event Structure

A `MutationEvent` contains:
- **`type`**: The GraphQL Type name (e.g., "User").
- **`uid`**: The unique ID of the affected node.
- **`mutation`**: The type of operation (`CREATE`, `UPDATE`, `DELETE`).
- **`payload`**: The actual data that was changed (JSON).

## Subscribing to Changes

VardaDB provides a generic `event` subscription field.

### Basic Subscription
Subscribe to *all* events in the database:

```graphql
subscription {
    event {
        type
        uid
        mutation
        payload
    }
}
```

### Filtering by Type
You can filter events to only receive updates for specific types using the `types` argument.

```graphql
subscription {
    event(types: ["User", "Post"]) {
        type
        uid
        mutation
        payload
    }
}
```
*This listener will only trigger when a User or Post is modified.*

## Payload Handling

The `payload` field returns a generic `JSON` scalar containing the fields that were changed.

- **Create**: Contains the full initial state of the object.
- **Update**: Contains only the fields that were modified (partial update).
- **Delete**: Payload is usually empty or contains minimal info, as the node is gone.

## Use Cases

1.  **Live Feeds**: Update a news feed instantly when new Posts are created.
2.  **Collaborative Editing**: Reflect changes made by one user immediately to others.
3.  **Syncing**: Keep a local cache or frontend store in sync with the backend state.
