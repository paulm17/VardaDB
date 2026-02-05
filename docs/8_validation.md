# Validation and Custom Scalars

VardaDB goes beyond standard GraphQL types by providing a rich set of extended scalars and built-in validation directives. This allows you to enforce data integrity directly at the schema level without writing custom application logic.

## Validation Directives

You can attach these directives to fields in your schema to enforce constraints.

### @length
Restricts the length of a String.

```graphql
type User {
    username: String @length(min: 3, max: 20)
}
```
- `min`: Minimum character length.
- `max`: Maximum character length.

### @range
Restricts the range of a numeric value (Int or Float).

```graphql
type Product {
    rating: Float @range(min: 0.0, max: 5.0)
    quantity: Int @range(min: 1)
}
```
- `min`: Minimum permitted value.
- `max`: Maximum permitted value.

### @regex
Enforces a Regular Expression pattern on a String.

```graphql
type Item {
    sku: String @regex(pattern: "^[A-Z]{3}-[0-9]{5}$")
}
```
- `pattern`: The standard Regex pattern to match.

---

## Extended Scalars

VardaDB includes a library of pre-defined scalars that come with built-in validation logic. Use these types in your schema to automatically validate inputs.

### Network
- `EmailAddress`: Valid email format.
- `IP`: IPv4 or IPv6 address.
- `IPv4`: IPv4 address.
- `IPv6`: IPv6 address.
- `URL`: Valid URL.
- `MAC`: MAC address.
- `Port`: Network port (0-65535).

### Identifiers
- `UUID`: Standard UUID format.
- `ULID`: Sortable unique identifier.

### Numeric Constrains
- `PositiveInt`, `NegativeInt`, `NonPositiveInt`, `NonNegativeInt`
- `PositiveFloat`, `NegativeFloat`, `NonPositiveFloat`, `NonNegativeFloat`

### Date & Time
- `Date`: "YYYY-MM-DD"
- `Time`: "HH:MM:SS" (or with milliseconds)
- `DateTime`: ISO-8601 (Standard)

### Colors
- `RGB`: `rgb(r, g, b)`
- `RGBA`: `rgba(r, g, b, a)`
- `HSL`: `hsl(h, s, l)`
- `HSLA`: `hsla(h, s, l, a)`
- `HexColorCode`: `#RRGGBB` or `#RRGGBBAA`

### Miscellaneous
- `JSON`: Arbitrary JSON data.
- `CustomJson`: Validates input string is valid JSON.
- `CustomJsonObject`: Validates input string is a valid JSON Object.
- `Locale`: Locale string (e.g., `en-US`).
- `Currency`: ISO 4217 currency code (e.g., `USD`).
- `JWT`: JSON Web Token format.

## usage Example

```graphql
type UserProfile {
    email: EmailAddress
    website: URL
    ipAddress: IP
    favoriteColor: HexColorCode
    score: PositiveInt
}
```
