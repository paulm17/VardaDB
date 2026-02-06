declare const _default: {
    scalars: number[];
    types: {
        BooleanFilter: {
            eq: number[];
            __typename: number[];
        };
        Boolean: {};
        DateTime: {};
        DateTimeFilter: {
            eq: number[];
            gt: number[];
            lt: number[];
            ge: number[];
            le: number[];
            in: number[];
            __typename: number[];
        };
        Mutation: {
            createTodo: (number | {
                input: (string | number)[];
            })[];
            updateTodo: (number | {
                id: (string | number)[];
                input: (string | number)[];
            })[];
            deleteTodo: (number | {
                id: (string | number)[];
            })[];
            __typename: number[];
        };
        ID: {};
        MutationEvent: {
            type: number[];
            uid: number[];
            mutation: number[];
            __typename: number[];
        };
        String: {};
        MutationType: {};
        Query: {
            queryTodo: (number | {
                filter: number[];
                sort: number[];
                first: number[];
                after: number[];
            })[];
            getTodo: (number | {
                id: number[];
            })[];
            __typename: number[];
        };
        Int: {};
        SortDirection: {};
        StringFilter: {
            eq: number[];
            contains: number[];
            allofterms: number[];
            anyofterms: number[];
            alloftext: number[];
            anyoftext: number[];
            lt: number[];
            le: number[];
            gt: number[];
            ge: number[];
            in: number[];
            __typename: number[];
        };
        Subscription: {
            event: (number | {
                types: (string | number)[];
            })[];
            __typename: number[];
        };
        Todo: {
            uid: number[];
            id: number[];
            title: number[];
            completed: number[];
            createdAt: number[];
            __typename: number[];
        };
        TodoFilter: {
            title: number[];
            completed: number[];
            createdAt: number[];
            and: number[];
            or: number[];
            not: number[];
            __typename: number[];
        };
        TodoInput: {
            uid: number[];
            title: number[];
            completed: number[];
            createdAt: number[];
            __typename: number[];
        };
        TodoSort: {
            title: number[];
            completed: number[];
            createdAt: number[];
            __typename: number[];
        };
    };
};
export default _default;
