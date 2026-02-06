import { Client, fetchExchange, subscriptionExchange, useQuery, useSubscription, useClient, Provider } from 'urql';
import { createClient as createWSClient } from 'graphql-ws';
import { cacheExchange as graphCacheExchange } from '@urql/exchange-graphcache';
import { useMemo } from 'react';
import { generateQueryOp, generateMutationOp, generateSubscriptionOp } from './genql';
// --- 1. Client Factory ---
export const createVardaClient = (options) => {
    // Convert HTTP to WS if needed, or assume caller passes base
    // If user passes "http://...", we replace with "ws://" for subscription?
    // Let's assume input is http
    const httpUrl = options.url;
    const wsUrl = httpUrl.replace(/^http/, 'ws');
    const wsClient = createWSClient({ url: wsUrl });
    return new Client({
        url: httpUrl,
        exchanges: [
            graphCacheExchange({
                keys: { MutationEvent: () => null }, // Custom Cache Keys
                updates: {
                    Subscription: {
                        event: (_result, _args, cache, _info) => {
                            cache.invalidate("Query", "queryTodo"); // Auto-invalidates list
                        }
                    }
                }
            }),
            fetchExchange,
            subscriptionExchange({
                forwardSubscription: (operation) => ({
                    subscribe: (sink) => ({
                        unsubscribe: wsClient.subscribe({ ...operation, query: operation.query || '' }, sink),
                    }),
                }),
            }),
        ],
    });
};
// --- 2. Public Exports ---
export { Provider as VardaProvider };
/**
 * useVardaQuery: Type-safe data fetching with Urql Cache
 */
export function useVardaQuery(request, args) {
    const queryOp = useMemo(() => {
        try {
            return generateQueryOp(request);
        }
        catch (e) {
            console.error("Varda Genql Error", e);
            return { query: '', variables: {} };
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [JSON.stringify(request)]);
    return useQuery({
        query: queryOp.query,
        variables: queryOp.variables,
        ...args
    });
}
/**
 * useVardaSubscription: Type-safe subscriptions
 */
export function useVardaSubscription(request, handler) {
    const subOp = useMemo(() => {
        try {
            return generateSubscriptionOp(request);
        }
        catch (e) {
            return { query: '', variables: {} };
        }
    }, [JSON.stringify(request)]);
    return useSubscription({
        query: subOp.query,
        variables: subOp.variables
    }, handler);
}
/**
 * useVarda: One-stop shop for Mutations and Subscriptions
 */
export function useVarda() {
    const client = useClient();
    return {
        query: (request) => {
            const { query, variables } = generateQueryOp(request);
            return client.query(query, variables).toPromise();
        },
        mutation: async (request) => {
            const { query, variables } = generateMutationOp(request);
            return client.mutation(query, variables).toPromise();
        },
        subscription: (request) => {
            const { query, variables } = generateSubscriptionOp(request);
            return client.subscription(query, variables);
        }
    };
}
