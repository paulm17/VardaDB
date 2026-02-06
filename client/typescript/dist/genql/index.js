import { linkTypeMap, createClient as createClientOriginal, generateGraphqlOperation, GenqlError, } from './runtime';
export { GenqlError };
import types from './types';
export * from './schema';
const typeMap = linkTypeMap(types);
export const createClient = function (options) {
    return createClientOriginal({
        url: undefined,
        ...options,
        queryRoot: typeMap.Query,
        mutationRoot: typeMap.Mutation,
        subscriptionRoot: typeMap.Subscription,
    });
};
export const everything = {
    __scalar: true,
};
export const generateQueryOp = function (fields) {
    return generateGraphqlOperation('query', typeMap.Query, fields);
};
export const generateMutationOp = function (fields) {
    return generateGraphqlOperation('mutation', typeMap.Mutation, fields);
};
export const generateSubscriptionOp = function (fields) {
    return generateGraphqlOperation('subscription', typeMap.Subscription, fields);
};
