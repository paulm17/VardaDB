export default {
    "scalars": [
        1,
        2,
        5,
        7,
        8,
        10,
        11
    ],
    "types": {
        "BooleanFilter": {
            "eq": [
                1
            ],
            "__typename": [
                7
            ]
        },
        "Boolean": {},
        "DateTime": {},
        "DateTimeFilter": {
            "eq": [
                2
            ],
            "gt": [
                2
            ],
            "lt": [
                2
            ],
            "ge": [
                2
            ],
            "le": [
                2
            ],
            "in": [
                2
            ],
            "__typename": [
                7
            ]
        },
        "Mutation": {
            "createTodo": [
                14,
                {
                    "input": [
                        16,
                        "TodoInput!"
                    ]
                }
            ],
            "updateTodo": [
                1,
                {
                    "id": [
                        5,
                        "ID!"
                    ],
                    "input": [
                        16,
                        "TodoInput!"
                    ]
                }
            ],
            "deleteTodo": [
                1,
                {
                    "id": [
                        5,
                        "ID!"
                    ]
                }
            ],
            "__typename": [
                7
            ]
        },
        "ID": {},
        "MutationEvent": {
            "type": [
                7
            ],
            "uid": [
                5
            ],
            "mutation": [
                8
            ],
            "__typename": [
                7
            ]
        },
        "String": {},
        "MutationType": {},
        "Query": {
            "queryTodo": [
                14,
                {
                    "filter": [
                        15
                    ],
                    "sort": [
                        17
                    ],
                    "first": [
                        10
                    ],
                    "after": [
                        7
                    ]
                }
            ],
            "getTodo": [
                14,
                {
                    "id": [
                        5
                    ]
                }
            ],
            "__typename": [
                7
            ]
        },
        "Int": {},
        "SortDirection": {},
        "StringFilter": {
            "eq": [
                7
            ],
            "contains": [
                7
            ],
            "allofterms": [
                7
            ],
            "anyofterms": [
                7
            ],
            "alloftext": [
                7
            ],
            "anyoftext": [
                7
            ],
            "lt": [
                7
            ],
            "le": [
                7
            ],
            "gt": [
                7
            ],
            "ge": [
                7
            ],
            "in": [
                7
            ],
            "__typename": [
                7
            ]
        },
        "Subscription": {
            "event": [
                6,
                {
                    "types": [
                        7,
                        "[String]"
                    ]
                }
            ],
            "__typename": [
                7
            ]
        },
        "Todo": {
            "uid": [
                5
            ],
            "id": [
                5
            ],
            "title": [
                7
            ],
            "completed": [
                1
            ],
            "createdAt": [
                2
            ],
            "__typename": [
                7
            ]
        },
        "TodoFilter": {
            "title": [
                12
            ],
            "completed": [
                0
            ],
            "createdAt": [
                3
            ],
            "and": [
                15
            ],
            "or": [
                15
            ],
            "not": [
                15
            ],
            "__typename": [
                7
            ]
        },
        "TodoInput": {
            "uid": [
                5
            ],
            "title": [
                7
            ],
            "completed": [
                1
            ],
            "createdAt": [
                2
            ],
            "__typename": [
                7
            ]
        },
        "TodoSort": {
            "title": [
                11
            ],
            "completed": [
                11
            ],
            "createdAt": [
                11
            ],
            "__typename": [
                7
            ]
        }
    }
};
