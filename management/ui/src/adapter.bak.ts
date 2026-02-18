console.log("Initializing adapter module");
import { BrowserAdapter } from "./adapter/browser";

export class VardaDBAdapter {
    async query(query: string, vars?: any): Promise<any> {
        console.log("Executing VardaDB Query:", query, vars);
        return [{ status: "OK", time: "0ms", result: [] }];
    }

    async use(ns: string, db: string) {
        console.log(`Using NS: ${ns}, DB: ${db}`);
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
}

export const adapter = new BrowserAdapter();
export const surreal = new VardaDBAdapter();
export const isDesktop = false;
export const isMini = false;
