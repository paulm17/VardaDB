import { Client, type UseQueryArgs, Provider } from 'urql';
import { type QueryGenqlSelection as QueryRequest, type MutationGenqlSelection as MutationRequest, type SubscriptionGenqlSelection as SubscriptionRequest } from './genql';
export declare const createVardaClient: (options: {
    url: string;
}) => Client;
export { Provider as VardaProvider };
export type { QueryRequest, MutationRequest, SubscriptionRequest };
/**
 * useVardaQuery: Type-safe data fetching with Urql Cache
 */
export declare function useVardaQuery<Result = any>(request: QueryRequest, args?: Omit<UseQueryArgs, 'query'>): import("urql").UseQueryResponse<Result, import("urql").AnyVariables>;
/**
 * useVardaSubscription: Type-safe subscriptions
 */
export declare function useVardaSubscription<Result = any>(request: SubscriptionRequest, handler?: (summary: any, event: Result) => any): import("urql").UseSubscriptionResponse<Result, import("urql").AnyVariables>;
/**
 * useVarda: One-stop shop for Mutations and Subscriptions
 */
export declare function useVarda(): {
    query: (request: QueryRequest) => Promise<import("urql").OperationResult<any, {
        [name: string]: any;
    } | undefined>>;
    mutation: (request: MutationRequest) => Promise<import("urql").OperationResult<any, {
        [name: string]: any;
    } | undefined>>;
    subscription: (request: SubscriptionRequest) => import("urql").OperationResultSource<import("urql").OperationResult<any, {
        [name: string]: any;
    } | undefined>>;
};
