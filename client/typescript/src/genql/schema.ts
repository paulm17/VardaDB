// @ts-nocheck
/* istanbul ignore file */
/* tslint:disable */
/* eslint-disable */

export type Scalars = {
    Boolean: boolean,
    DateTime: any,
    ID: string,
    String: string,
    Int: number,
}

export interface Mutation {
    createTodo: (Todo | null)
    updateTodo: (Scalars['Boolean'] | null)
    deleteTodo: (Scalars['Boolean'] | null)
    __typename: 'Mutation'
}

export interface MutationEvent {
    type: Scalars['String']
    uid: Scalars['ID']
    mutation: MutationType
    __typename: 'MutationEvent'
}

export type MutationType = 'CREATE' | 'UPDATE' | 'DELETE'

export interface Query {
    queryTodo: ((Todo | null)[] | null)
    getTodo: (Todo | null)
    __typename: 'Query'
}

export type SortDirection = 'ASC' | 'DESC'

export interface Subscription {
    event: MutationEvent
    __typename: 'Subscription'
}

export interface Todo {
    uid: Scalars['ID']
    id: Scalars['ID']
    title: (Scalars['String'] | null)
    completed: (Scalars['Boolean'] | null)
    createdAt: (Scalars['DateTime'] | null)
    __typename: 'Todo'
}

export interface BooleanFilter {eq?: (Scalars['Boolean'] | null)}

export interface DateTimeFilter {eq?: (Scalars['DateTime'] | null),gt?: (Scalars['DateTime'] | null),lt?: (Scalars['DateTime'] | null),ge?: (Scalars['DateTime'] | null),le?: (Scalars['DateTime'] | null),in?: ((Scalars['DateTime'] | null)[] | null)}

export interface MutationGenqlSelection{
    createTodo?: (TodoGenqlSelection & { __args: {input: TodoInput} })
    updateTodo?: { __args: {id: Scalars['ID'], input: TodoInput} }
    deleteTodo?: { __args: {id: Scalars['ID']} }
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface MutationEventGenqlSelection{
    type?: boolean | number
    uid?: boolean | number
    mutation?: boolean | number
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface QueryGenqlSelection{
    queryTodo?: (TodoGenqlSelection & { __args?: {filter?: (TodoFilter | null), sort?: (TodoSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getTodo?: (TodoGenqlSelection & { __args?: {id?: (Scalars['ID'] | null)} })
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface StringFilter {eq?: (Scalars['String'] | null),contains?: (Scalars['String'] | null),allofterms?: (Scalars['String'] | null),anyofterms?: (Scalars['String'] | null),alloftext?: (Scalars['String'] | null),anyoftext?: (Scalars['String'] | null),lt?: (Scalars['String'] | null),le?: (Scalars['String'] | null),gt?: (Scalars['String'] | null),ge?: (Scalars['String'] | null),in?: ((Scalars['String'] | null)[] | null)}

export interface SubscriptionGenqlSelection{
    event?: (MutationEventGenqlSelection & { __args?: {types?: ((Scalars['String'] | null)[] | null)} })
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface TodoGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    title?: boolean | number
    completed?: boolean | number
    createdAt?: boolean | number
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface TodoFilter {title?: (StringFilter | null),completed?: (BooleanFilter | null),createdAt?: (DateTimeFilter | null),and?: ((TodoFilter | null)[] | null),or?: ((TodoFilter | null)[] | null),not?: (TodoFilter | null)}

export interface TodoInput {uid?: (Scalars['ID'] | null),title?: (Scalars['String'] | null),completed?: (Scalars['Boolean'] | null),createdAt?: (Scalars['DateTime'] | null)}

export interface TodoSort {title?: (SortDirection | null),completed?: (SortDirection | null),createdAt?: (SortDirection | null)}


    const Mutation_possibleTypes: string[] = ['Mutation']
    export const isMutation = (obj?: { __typename?: any } | null): obj is Mutation => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isMutation"')
      return Mutation_possibleTypes.includes(obj.__typename)
    }
    


    const MutationEvent_possibleTypes: string[] = ['MutationEvent']
    export const isMutationEvent = (obj?: { __typename?: any } | null): obj is MutationEvent => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isMutationEvent"')
      return MutationEvent_possibleTypes.includes(obj.__typename)
    }
    


    const Query_possibleTypes: string[] = ['Query']
    export const isQuery = (obj?: { __typename?: any } | null): obj is Query => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isQuery"')
      return Query_possibleTypes.includes(obj.__typename)
    }
    


    const Subscription_possibleTypes: string[] = ['Subscription']
    export const isSubscription = (obj?: { __typename?: any } | null): obj is Subscription => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isSubscription"')
      return Subscription_possibleTypes.includes(obj.__typename)
    }
    


    const Todo_possibleTypes: string[] = ['Todo']
    export const isTodo = (obj?: { __typename?: any } | null): obj is Todo => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isTodo"')
      return Todo_possibleTypes.includes(obj.__typename)
    }
    

export const enumMutationType = {
   CREATE: 'CREATE' as const,
   UPDATE: 'UPDATE' as const,
   DELETE: 'DELETE' as const
}

export const enumSortDirection = {
   ASC: 'ASC' as const,
   DESC: 'DESC' as const
}
