// @ts-nocheck
/* istanbul ignore file */
/* tslint:disable */
/* eslint-disable */
const Mutation_possibleTypes = ['Mutation'];
export const isMutation = (obj) => {
    if (!obj?.__typename)
        throw new Error('__typename is missing in "isMutation"');
    return Mutation_possibleTypes.includes(obj.__typename);
};
const MutationEvent_possibleTypes = ['MutationEvent'];
export const isMutationEvent = (obj) => {
    if (!obj?.__typename)
        throw new Error('__typename is missing in "isMutationEvent"');
    return MutationEvent_possibleTypes.includes(obj.__typename);
};
const Query_possibleTypes = ['Query'];
export const isQuery = (obj) => {
    if (!obj?.__typename)
        throw new Error('__typename is missing in "isQuery"');
    return Query_possibleTypes.includes(obj.__typename);
};
const Subscription_possibleTypes = ['Subscription'];
export const isSubscription = (obj) => {
    if (!obj?.__typename)
        throw new Error('__typename is missing in "isSubscription"');
    return Subscription_possibleTypes.includes(obj.__typename);
};
const Todo_possibleTypes = ['Todo'];
export const isTodo = (obj) => {
    if (!obj?.__typename)
        throw new Error('__typename is missing in "isTodo"');
    return Todo_possibleTypes.includes(obj.__typename);
};
export const enumMutationType = {
    CREATE: 'CREATE',
    UPDATE: 'UPDATE',
    DELETE: 'DELETE'
};
export const enumSortDirection = {
    ASC: 'ASC',
    DESC: 'DESC'
};
