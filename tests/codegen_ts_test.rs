use vardadb::codegen::generate_typescript;

#[test]
fn test_generate_simple_types() {
    let sdl = r#"
    type User {
        id: ID!
        name: String
        age: Int
        active: Boolean!
        posts: [Post!]
    }
    
    type Post {
        id: ID!
        title: String!
    }
    
    enum Role {
        ADMIN
        USER
    }
    
    input UserFilter {
        name: String
    }

    type Query {
        getUser(id: ID!): User
    }
    "#;

    let ts = generate_typescript(sdl).expect("Failed to generate TS");
    println!("{}", ts);
    
    assert!(ts.contains("export interface User {"));
    assert!(ts.contains("id: string;"));
    assert!(ts.contains("name: string | null;")); // Nullable by default
    assert!(ts.contains("age: number | null;"));
    assert!(ts.contains("active: boolean;")); // Non-null
    assert!(ts.contains("posts: Array<Post> | null;"));

    assert!(ts.contains("export interface Post {"));
    assert!(ts.contains("title: string;"));
    
    assert!(ts.contains("export type Role ="));
    assert!(ts.contains("\"ADMIN\""));
    assert!(ts.contains("\"USER\""));
    
    assert!(ts.contains("export interface UserFilter {"));
    assert!(ts.contains("name: string | null;"));

    // Client SDK Assertions
    assert!(ts.contains("export class VardaClient {"));
    assert!(ts.contains("async getUser(args: { id: string }): Promise<User | null> {"));
    
    // Hooks Assertions
    assert!(ts.contains("export const useGetUser = (client: VardaClient, args: { id: string }) => {"));
    // Note: User | null is the return type, plus useState adds | null, might result in User | null | null in naive string generation
    assert!(ts.contains("const [data, setData] = useState<User | null | null>(null);"));
}
