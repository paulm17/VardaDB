
import { BrowserAdapter } from "./adapter/browser";

export class VardaDBAdapter {
    private currentNs?: string = "default";
    private currentDb?: string = "default";

    private tableStructureCache: Record<string, any> = {};

    async query(query: string, vars?: any): Promise<any> {
        console.log(`Executing VardaDB Query [${this.currentNs}/${this.currentDb}]:`, query, vars);

        const queries = query.split(';').map(q => q.trim()).filter(q => q.length > 0);
        const results: any[] = [];

        for (const qStr of queries) {
            const q = qStr.toUpperCase();

            if (q.startsWith("INFO FOR NS STRUCTURE")) {
                const databases = await this.fetchDatabases();
                results.push({
                    status: "OK",
                    time: "0ms",
                    result: {
                        databases,
                        accesses: [],
                        users: []
                    }
                });
                continue;
            }

            if (q.startsWith("INFO FOR KV STRUCTURE")) {
                results.push({
                    status: "OK",
                    time: "0ms",
                    result: {
                        namespaces: [{ name: "default" }, { name: "sandbox" }],
                        accesses: [],
                        users: []
                    }
                });
                continue;
            }

            if (q.startsWith("INFO FOR DB STRUCTURE")) {
                results.push({
                    status: "OK",
                    time: "0ms",
                    result: {
                        accesses: [],
                        models: [],
                        users: [],
                        functions: [],
                        tables: await this.fetchTables(),
                        params: []
                    }
                });
                continue;
            }

            if (q.startsWith("INFO FOR TABLE")) {
                // Parse table name: INFO FOR TABLE <name> STRUCTURE
                const parts = qStr.split(/\s+/);
                const tableNameIndex = parts.findIndex(p => p.toUpperCase() === "TABLE") + 1;

                if (tableNameIndex > 0 && tableNameIndex < parts.length) {
                    const tableName = parts[tableNameIndex].replace(';', '');

                    const tableInfo = await this.fetchTableStructure(tableName);
                    results.push({
                        status: "OK",
                        time: "0ms",
                        result: tableInfo
                    });
                } else {
                    results.push({
                        status: "ERR",
                        time: "0ms",
                        result: "Invalid INFO FOR TABLE syntax"
                    });
                }
                continue;
            }

            // Handle SELECT count() (Pagination)
            // Regex: SELECT count() AS count FROM <Table> ...
            const countMatch = qStr.match(/SELECT\s+count\(\)\s+AS\s+count\s+FROM\s+(\w+)/i);
            if (countMatch) {
                // Mock count to support pagination (return 100 for now or implement actual count if possible)
                // TODO: Implement actual count aggregation via GraphQL if available
                results.push({
                    status: "OK",
                    time: "0ms",
                    result: [{ count: 100 }]
                });
                continue;
            }

            // Handle SELECT * FROM <Table> LIMIT <N>
            const selectMatch = qStr.match(/SELECT\s+\*\s+FROM\s+(\w+)(?:\s+LIMIT\s+(\d+))?/i);
            if (selectMatch) {
                const tableName = selectMatch[1];
                const limit = selectMatch[2] ? parseInt(selectMatch[2]) : 25;

                // Construct GraphQL Query
                // We need fields. Use cache. Use only Scalar or Enum fields to avoid "must have selection of subfields" error
                let fields = this.tableStructureCache[tableName]?.fields
                    ?.filter((f: any) => f.isScalarOrEnum || f.name === "id")
                    ?.map((f: any) => f.name) || ["id"];

                // If cache miss (shouldn't happen if INFO called first, but just in case), try to fetch (or default to id)
                if (!this.tableStructureCache[tableName]) {
                    // Try to fetch structure on the fly?
                    await this.fetchTableStructure(tableName);
                    if (this.tableStructureCache[tableName]) {
                        fields = this.tableStructureCache[tableName].fields
                            .filter((f: any) => f.isScalarOrEnum || f.name === "id")
                            .map((f: any) => f.name);
                    }
                }

                // Construct Query: query<Type>(first: N) { ... }
                // Assumption: Query name is `query<Type>`
                const gqlQuery = `query {
                    query${tableName}(first: ${limit}) {
                        ${fields.join('\n')}
                    }
                }`;

                try {
                    const response = await fetch('/graphql', {
                        method: 'POST',
                        headers: {
                            'Content-Type': 'application/json',
                            'x-varda-db': this.currentDb || "default"
                        },
                        body: JSON.stringify({ query: gqlQuery }),
                    });

                    const data = await response.json();

                    if (data.errors) {
                        results.push({ status: "ERR", time: "0ms", result: data.errors.map((e: any) => e.message).join(", ") });
                    } else {
                        // Result: data.queryBook -> [ ... ]
                        const resultKey = `query${tableName}`;
                        const records = data.data?.[resultKey] || [];
                        results.push({ status: "OK", time: "0ms", result: records });
                    }
                } catch (e: any) {
                    results.push({ status: "ERR", time: "0ms", result: e.message });
                }
                continue;
            }

            // Detect GraphQL query (basic heuristic)
            if (q.startsWith("QUERY") || q.startsWith("MUTATION") || q.startsWith("{")) {
                try {
                    const response = await fetch('/graphql', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ query: qStr, variables: vars }),
                    });

                    if (!response.ok) {
                        throw new Error(`GraphQL Error: ${response.statusText}`);
                    }

                    const data = await response.json();

                    if (data.errors) {
                        results.push({
                            status: "ERR",
                            time: "0ms",
                            result: data.errors.map((e: any) => e.message).join(", ")
                        });
                    } else {
                        results.push({
                            status: "OK",
                            time: "0ms",
                            result: data.data
                        });
                    }

                } catch (err: any) {
                    results.push({
                        status: "ERR",
                        time: "0ms",
                        result: err.message
                    });
                }
                continue;
            }

            // Define Namespace/Database fallbacks (ignore them successfully)
            if (q.startsWith("DEFINE NAMESPACE") || q.startsWith("DEFINE DATABASE")) {
                results.push({
                    status: "OK",
                    time: "0ms",
                    result: null
                });
                continue;
            }

            // Fallback
            results.push({
                status: "ERR",
                time: "0ms",
                result: "Only GraphQL queries are supported in this adapter. Please start your query with 'query', 'mutation', or '{'."
            });
        }

        return results;
    }

    async use(ns: string | { namespace?: string; database?: string }, db?: string) {
        if (typeof ns === "object") {
            this.currentNs = ns.namespace ?? this.currentNs;
            this.currentDb = ns.database ?? this.currentDb;
        } else {
            this.currentNs = ns;
            this.currentDb = db;
        }
        console.log(`Using NS: ${this.currentNs}, DB: ${this.currentDb}`);
        return { namespace: this.currentNs, database: this.currentDb };
    }

    async info() {
        return { user: "root", pass: "root" };
    }

    async signin(params: any) {
        console.log("Signing in:", params);
        return "mock-token";
    }

    async signup(params: any) {
        console.log("Signing up:", params);
        return "mock-token";
    }

    async invalidate() {
        console.log("Invalidating session");
    }

    async authenticate(token: string) {
        console.log("Authenticating with token:", token);
    }

    async connect(url: string, options?: any) {
        console.log("Connecting to:", url, options);
    }

    async close() {
        console.log("Closing connection");
    }

    // Mock event emitter methods
    subscribe(event: string, callback: any) {
        console.log("Subscribed to:", event);
        return { close: () => { } };
    }

    async version() {
        return { version: "surrealdb-2.0.0" };
    }
    async fetchDatabases() {
        try {
            console.log("Fetching databases from /management/db...");
            const response = await fetch('/management/db');

            if (!response.ok) {
                console.warn("Failed to fetch databases from /management/db", response.status);
                throw new Error(response.statusText);
            }

            const data = await response.json();
            console.log("Management API Database Response:", data);

            // Response format: { databases: ["name1", "name2"] }
            if (data && Array.isArray(data.databases)) {
                return data.databases.map((name: string) => ({ name }));
            }

            return [{ name: "default" }];
        } catch (error) {
            console.error("Failed to fetch databases", error);
            return [{ name: "default" }, { name: "sandbox" }];
        }
    }
    async fetchTables() {
        try {
            console.log("Fetching tables (types) from /graphql...");
            const query = `{
                __schema {
                    types {
                        name
                        kind
                    }
                }
            }`;

            const response = await fetch('/graphql', {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'x-varda-db': this.currentDb || "default"
                },
                body: JSON.stringify({ query }),
            });

            if (!response.ok) {
                console.warn("Failed to fetch tables from /graphql", response.status);
                return [];
            }

            const data = await response.json();
            const types = data.data?.__schema?.types || [];

            return types
                .filter((t: any) => t.kind === "OBJECT" && !t.name.startsWith("__") && t.name !== "Query" && t.name !== "Mutation" && t.name !== "Subscription")
                .map((t: any) => ({
                    name: t.name,
                    drop: false,
                    full: true,
                    permissions: {
                        select: true,
                        create: true,
                        update: true,
                        delete: true
                    },
                    kind: {
                        kind: "NORMAL",
                        enforced: true
                    }
                }));

        } catch (error) {
            console.error("Failed to fetch tables", error);
            return [];
        }
    }

    async fetchTableStructure(tableName: string) {
        try {
            console.log(`Fetching table structure for ${tableName}...`);
            const query = `{
                __type(name: "${tableName}") {
                    fields {
                        name
                        type {
                            name
                            kind
                            ofType {
                                name
                                kind
                            }
                        }
                    }
                }
            }`;

            const response = await fetch('/graphql', {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'x-varda-db': this.currentDb || "default"
                },
                body: JSON.stringify({ query }),
            });

            if (!response.ok) {
                return { fields: [], indexes: [], events: [], tables: [] };
            }

            const data = await response.json();
            const fields = data.data?.__type?.fields || [];

            // Map GraphQL fields to SurrealDB SchemaFields
            const schemaFields = fields.map((f: any) => {
                let typeName = f.type.name;
                let kind = f.type.kind;
                let isScalarOrEnum = kind === "SCALAR" || kind === "ENUM";

                if (!typeName && kind === "NON_NULL") {
                    typeName = f.type.ofType?.name;
                    kind = f.type.ofType?.kind;
                    isScalarOrEnum = kind === "SCALAR" || kind === "ENUM";
                }

                if (kind === "LIST") {
                    typeName = "array"; // Simplified
                    // Check if list of scalars
                    const innerKind = f.type.ofType?.kind || f.type.ofType?.ofType?.kind;
                    isScalarOrEnum = innerKind === "SCALAR" || innerKind === "ENUM";
                }

                return {
                    name: f.name,
                    flex: false,
                    readonly: false,
                    kind: typeName || "any",
                    permissions: {
                        select: true,
                        create: true,
                        update: true,
                        delete: true
                    },
                    // Custom property to help with Query generation
                    isScalarOrEnum: isScalarOrEnum
                };
            });

            const info = {
                fields: schemaFields,
                indexes: [],
                events: [],
                tables: [] // Sub-tables not supported yet
            };

            this.tableStructureCache[tableName] = info;
            return info;

        } catch (error) {
            console.error("Failed to fetch table structure", error);
            return { fields: [], indexes: [], events: [], tables: [] };
        }
    }
}

export const adapter = new BrowserAdapter();
export const surreal = new VardaDBAdapter();
export const isDesktop = false;
export const isMini = false;
