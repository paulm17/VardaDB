import { Client, fetchExchange, subscriptionExchange, useQuery, useSubscription, useClient, type UseQueryArgs, Provider } from 'urql';
import { createClient as createWSClient } from 'graphql-ws';
import { cacheExchange as graphCacheExchange } from '@urql/exchange-graphcache';
import { useMemo } from 'react';
import {
    generateQueryOp,
    generateMutationOp,
    generateSubscriptionOp,
    type QueryGenqlSelection as QueryRequest,
    type MutationGenqlSelection as MutationRequest,
    type SubscriptionGenqlSelection as SubscriptionRequest
} from './genql';


// --- 1. Client Factory ---
export const createVardaClient = (options: { url: string; database?: string }) => {
    // Convert HTTP to WS if needed, or assume caller passes base
    // If user passes "http://...", we replace with "ws://" for subscription?
    // Let's assume input is http
    const httpUrl = options.url;
    const wsUrl = httpUrl.replace(/^http/, 'ws');
    const database = options.database || 'default';

    const wsClient = createWSClient({ 
        url: wsUrl,
        connectionParams: {
            headers: {
                'x-varda-db': database
            }
        }
    });

    return new Client({
        url: httpUrl,
        fetchOptions: () => {
            return {
                headers: {
                    'x-varda-db': database
                }
            };
        },
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
export type { QueryRequest, MutationRequest, SubscriptionRequest };

/**
 * useVardaQuery: Type-safe data fetching with Urql Cache
 */
export function useVardaQuery<Result = any>(
    request: QueryRequest,
    args?: Omit<UseQueryArgs, 'query'>
) {
    const queryOp = useMemo(() => {
        try {
            return generateQueryOp(request);
        } catch (e) {
            console.error("Varda Genql Error", e);
            return { query: '', variables: {} };
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [JSON.stringify(request)]);

    return useQuery<Result>({
        query: queryOp.query,
        variables: queryOp.variables,
        ...args
    });
}

/**
 * useVardaSubscription: Type-safe subscriptions
 */
export function useVardaSubscription<Result = any>(
    request: SubscriptionRequest,
    handler?: (summary: any, event: Result) => any
) {
    const subOp = useMemo(() => {
        try {
            return generateSubscriptionOp(request);
        } catch (e) {
            return { query: '', variables: {} };
        }
    }, [JSON.stringify(request)]);

    return useSubscription<Result>({
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
        query: (request: QueryRequest) => {
            const { query, variables } = generateQueryOp(request);
            return client.query(query, variables).toPromise();
        },
        mutation: async (request: MutationRequest) => {
            const { query, variables } = generateMutationOp(request);
            return client.mutation(query, variables).toPromise();
        },
        subscription: (request: SubscriptionRequest) => {
            const { query, variables } = generateSubscriptionOp(request);
            return client.subscription(query, variables);
        }
    }
}
