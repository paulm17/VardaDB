export default {
    "scalars": [
        1,
        2,
        3,
        16,
        25,
        32,
        46,
        65,
        68,
        111
    ],
    "types": {
        "Book": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "code": [
                2
            ],
            "nameEn": [
                2
            ],
            "nameHe": [
                2
            ],
            "nameGr": [
                2
            ],
            "chapters": [
                3
            ],
            "bookTranslations": [
                11
            ],
            "bookCategories": [
                4
            ],
            "summaries": [
                71
            ],
            "__typename": [
                2
            ]
        },
        "ID": {},
        "String": {},
        "Int": {},
        "BookCategory": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "book": [
                0
            ],
            "category": [
                17
            ],
            "__typename": [
                2
            ]
        },
        "BookCategoryFilter": {
            "and": [
                5
            ],
            "or": [
                5
            ],
            "not": [
                5
            ],
            "__typename": [
                2
            ]
        },
        "BookCategoryInput": {
            "uid": [
                1
            ],
            "book": [
                9
            ],
            "category": [
                19
            ],
            "__typename": [
                2
            ]
        },
        "BookCategorySort": {
            "id": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "BookFilter": {
            "code": [
                69
            ],
            "nameEn": [
                69
            ],
            "nameHe": [
                69
            ],
            "nameGr": [
                69
            ],
            "chapters": [
                45
            ],
            "and": [
                8
            ],
            "or": [
                8
            ],
            "not": [
                8
            ],
            "__typename": [
                2
            ]
        },
        "BookInput": {
            "uid": [
                1
            ],
            "code": [
                2
            ],
            "nameEn": [
                2
            ],
            "nameHe": [
                2
            ],
            "nameGr": [
                2
            ],
            "chapters": [
                3
            ],
            "bookTranslations": [
                13
            ],
            "bookCategories": [
                6
            ],
            "summaries": [
                73
            ],
            "__typename": [
                2
            ]
        },
        "BookSort": {
            "code": [
                68
            ],
            "nameEn": [
                68
            ],
            "nameHe": [
                68
            ],
            "nameGr": [
                68
            ],
            "chapters": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "BookTranslation": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "book": [
                0
            ],
            "translation": [
                83
            ],
            "chapters": [
                21,
                {
                    "sort": [
                        24
                    ]
                }
            ],
            "__typename": [
                2
            ]
        },
        "BookTranslationFilter": {
            "and": [
                12
            ],
            "or": [
                12
            ],
            "not": [
                12
            ],
            "__typename": [
                2
            ]
        },
        "BookTranslationInput": {
            "uid": [
                1
            ],
            "book": [
                9
            ],
            "translation": [
                85
            ],
            "chapters": [
                23
            ],
            "__typename": [
                2
            ]
        },
        "BookTranslationSort": {
            "id": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "BooleanFilter": {
            "eq": [
                16
            ],
            "__typename": [
                2
            ]
        },
        "Boolean": {},
        "Category": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "name": [
                2
            ],
            "bookCategories": [
                4
            ],
            "__typename": [
                2
            ]
        },
        "CategoryFilter": {
            "name": [
                69
            ],
            "and": [
                18
            ],
            "or": [
                18
            ],
            "not": [
                18
            ],
            "__typename": [
                2
            ]
        },
        "CategoryInput": {
            "uid": [
                1
            ],
            "name": [
                2
            ],
            "bookCategories": [
                6
            ],
            "__typename": [
                2
            ]
        },
        "CategorySort": {
            "name": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "Chapter": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "number": [
                3
            ],
            "bookTranslation": [
                11
            ],
            "verses": [
                91,
                {
                    "filter": [
                        100
                    ]
                }
            ],
            "summaries": [
                71
            ],
            "__typename": [
                2
            ]
        },
        "ChapterFilter": {
            "number": [
                45
            ],
            "and": [
                22
            ],
            "or": [
                22
            ],
            "not": [
                22
            ],
            "__typename": [
                2
            ]
        },
        "ChapterInput": {
            "uid": [
                1
            ],
            "number": [
                3
            ],
            "bookTranslation": [
                13
            ],
            "verses": [
                101
            ],
            "summaries": [
                73
            ],
            "__typename": [
                2
            ]
        },
        "ChapterSort": {
            "number": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "DateTime": {},
        "DateTimeFilter": {
            "eq": [
                25
            ],
            "gt": [
                25
            ],
            "lt": [
                25
            ],
            "ge": [
                25
            ],
            "le": [
                25
            ],
            "in": [
                25
            ],
            "__typename": [
                2
            ]
        },
        "Entities": {
            "uid": [
                1
            ],
            "entityId": [
                3
            ],
            "name": [
                2
            ],
            "entityType": [
                2
            ],
            "description": [
                2
            ],
            "mentions": [
                36
            ],
            "edgesA": [
                31
            ],
            "edgesB": [
                31
            ],
            "__typename": [
                2
            ]
        },
        "EntitiesFilter": {
            "entityId": [
                45
            ],
            "name": [
                69
            ],
            "entityType": [
                69
            ],
            "description": [
                69
            ],
            "and": [
                28
            ],
            "or": [
                28
            ],
            "not": [
                28
            ],
            "__typename": [
                2
            ]
        },
        "EntitiesInput": {
            "uid": [
                1
            ],
            "entityId": [
                3
            ],
            "name": [
                2
            ],
            "entityType": [
                2
            ],
            "description": [
                2
            ],
            "mentions": [
                38
            ],
            "edgesA": [
                34
            ],
            "edgesB": [
                34
            ],
            "__typename": [
                2
            ]
        },
        "EntitiesSort": {
            "entityId": [
                68
            ],
            "name": [
                68
            ],
            "entityType": [
                68
            ],
            "description": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "EntityEdges": {
            "uid": [
                1
            ],
            "edgeId": [
                3
            ],
            "relationType": [
                2
            ],
            "weight": [
                32
            ],
            "entityA": [
                27
            ],
            "entityB": [
                27
            ],
            "__typename": [
                2
            ]
        },
        "Float": {},
        "EntityEdgesFilter": {
            "edgeId": [
                45
            ],
            "relationType": [
                69
            ],
            "weight": [
                40
            ],
            "and": [
                33
            ],
            "or": [
                33
            ],
            "not": [
                33
            ],
            "__typename": [
                2
            ]
        },
        "EntityEdgesInput": {
            "uid": [
                1
            ],
            "edgeId": [
                3
            ],
            "relationType": [
                2
            ],
            "weight": [
                32
            ],
            "entityA": [
                29
            ],
            "entityB": [
                29
            ],
            "__typename": [
                2
            ]
        },
        "EntityEdgesSort": {
            "edgeId": [
                68
            ],
            "relationType": [
                68
            ],
            "weight": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "EntityMentions": {
            "uid": [
                1
            ],
            "mentionId": [
                3
            ],
            "tokenStart": [
                3
            ],
            "tokenEnd": [
                3
            ],
            "confidence": [
                32
            ],
            "entity": [
                27
            ],
            "verse": [
                91
            ],
            "__typename": [
                2
            ]
        },
        "EntityMentionsFilter": {
            "mentionId": [
                45
            ],
            "tokenStart": [
                45
            ],
            "tokenEnd": [
                45
            ],
            "confidence": [
                40
            ],
            "and": [
                37
            ],
            "or": [
                37
            ],
            "not": [
                37
            ],
            "__typename": [
                2
            ]
        },
        "EntityMentionsInput": {
            "uid": [
                1
            ],
            "mentionId": [
                3
            ],
            "tokenStart": [
                3
            ],
            "tokenEnd": [
                3
            ],
            "confidence": [
                32
            ],
            "entity": [
                29
            ],
            "verse": [
                101
            ],
            "__typename": [
                2
            ]
        },
        "EntityMentionsSort": {
            "mentionId": [
                68
            ],
            "tokenStart": [
                68
            ],
            "tokenEnd": [
                68
            ],
            "confidence": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "FloatFilter": {
            "eq": [
                32
            ],
            "gt": [
                32
            ],
            "lt": [
                32
            ],
            "ge": [
                32
            ],
            "le": [
                32
            ],
            "between": [
                32
            ],
            "in": [
                32
            ],
            "__typename": [
                2
            ]
        },
        "InterlinearAlignments": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "lemmaA": [
                55
            ],
            "lemmaB": [
                55
            ],
            "__typename": [
                2
            ]
        },
        "InterlinearAlignmentsFilter": {
            "and": [
                42
            ],
            "or": [
                42
            ],
            "not": [
                42
            ],
            "__typename": [
                2
            ]
        },
        "InterlinearAlignmentsInput": {
            "uid": [
                1
            ],
            "lemmaA": [
                57
            ],
            "lemmaB": [
                57
            ],
            "__typename": [
                2
            ]
        },
        "InterlinearAlignmentsSort": {
            "id": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "IntFilter": {
            "eq": [
                3
            ],
            "gt": [
                3
            ],
            "lt": [
                3
            ],
            "ge": [
                3
            ],
            "le": [
                3
            ],
            "between": [
                3
            ],
            "in": [
                3
            ],
            "__typename": [
                2
            ]
        },
        "JSON": {},
        "Keyword": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "text": [
                2
            ],
            "videos": [
                106
            ],
            "__typename": [
                2
            ]
        },
        "KeywordFilter": {
            "text": [
                69
            ],
            "and": [
                48
            ],
            "or": [
                48
            ],
            "not": [
                48
            ],
            "__typename": [
                2
            ]
        },
        "KeywordInput": {
            "uid": [
                1
            ],
            "text": [
                2
            ],
            "videos": [
                108
            ],
            "__typename": [
                2
            ]
        },
        "KeywordSort": {
            "text": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "Language": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "code": [
                2
            ],
            "name": [
                2
            ],
            "translations": [
                83
            ],
            "__typename": [
                2
            ]
        },
        "LanguageFilter": {
            "code": [
                69
            ],
            "name": [
                69
            ],
            "and": [
                52
            ],
            "or": [
                52
            ],
            "not": [
                52
            ],
            "__typename": [
                2
            ]
        },
        "LanguageInput": {
            "uid": [
                1
            ],
            "code": [
                2
            ],
            "name": [
                2
            ],
            "translations": [
                85
            ],
            "__typename": [
                2
            ]
        },
        "LanguageSort": {
            "code": [
                68
            ],
            "name": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "Lemmas": {
            "uid": [
                1
            ],
            "lemmaId": [
                3
            ],
            "strongsId": [
                2
            ],
            "languageCode": [
                2
            ],
            "lemmaText": [
                2
            ],
            "gloss": [
                2
            ],
            "wordnetLinks": [
                116
            ],
            "lexiconEmbeddings": [
                59
            ],
            "syntaxRelationsSub": [
                75
            ],
            "syntaxRelationsVerb": [
                75
            ],
            "syntaxRelationsObj": [
                75
            ],
            "interlinearAlignmentsA": [
                41
            ],
            "interlinearAlignmentsB": [
                41
            ],
            "__typename": [
                2
            ]
        },
        "LemmasFilter": {
            "lemmaId": [
                45
            ],
            "strongsId": [
                69
            ],
            "languageCode": [
                69
            ],
            "lemmaText": [
                69
            ],
            "gloss": [
                69
            ],
            "and": [
                56
            ],
            "or": [
                56
            ],
            "not": [
                56
            ],
            "__typename": [
                2
            ]
        },
        "LemmasInput": {
            "uid": [
                1
            ],
            "lemmaId": [
                3
            ],
            "strongsId": [
                2
            ],
            "languageCode": [
                2
            ],
            "lemmaText": [
                2
            ],
            "gloss": [
                2
            ],
            "wordnetLinks": [
                118
            ],
            "lexiconEmbeddings": [
                61
            ],
            "syntaxRelationsSub": [
                77
            ],
            "syntaxRelationsVerb": [
                77
            ],
            "syntaxRelationsObj": [
                77
            ],
            "interlinearAlignmentsA": [
                43
            ],
            "interlinearAlignmentsB": [
                43
            ],
            "__typename": [
                2
            ]
        },
        "LemmasSort": {
            "lemmaId": [
                68
            ],
            "strongsId": [
                68
            ],
            "languageCode": [
                68
            ],
            "lemmaText": [
                68
            ],
            "gloss": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "LexiconEmbeddings": {
            "uid": [
                1
            ],
            "embeddingId": [
                3
            ],
            "modelName": [
                2
            ],
            "lemma": [
                55
            ],
            "__typename": [
                2
            ]
        },
        "LexiconEmbeddingsFilter": {
            "embeddingId": [
                45
            ],
            "modelName": [
                69
            ],
            "and": [
                60
            ],
            "or": [
                60
            ],
            "not": [
                60
            ],
            "__typename": [
                2
            ]
        },
        "LexiconEmbeddingsInput": {
            "uid": [
                1
            ],
            "embeddingId": [
                3
            ],
            "modelName": [
                2
            ],
            "lemma": [
                57
            ],
            "__typename": [
                2
            ]
        },
        "LexiconEmbeddingsSort": {
            "embeddingId": [
                68
            ],
            "modelName": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "Mutation": {
            "createLanguage": [
                51,
                {
                    "input": [
                        53,
                        "LanguageInput!"
                    ]
                }
            ],
            "updateLanguage": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        53,
                        "LanguageInput!"
                    ]
                }
            ],
            "deleteLanguage": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createCategory": [
                17,
                {
                    "input": [
                        19,
                        "CategoryInput!"
                    ]
                }
            ],
            "updateCategory": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        19,
                        "CategoryInput!"
                    ]
                }
            ],
            "deleteCategory": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createTranslations": [
                83,
                {
                    "input": [
                        85,
                        "TranslationsInput!"
                    ]
                }
            ],
            "updateTranslations": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        85,
                        "TranslationsInput!"
                    ]
                }
            ],
            "deleteTranslations": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createBook": [
                0,
                {
                    "input": [
                        9,
                        "BookInput!"
                    ]
                }
            ],
            "updateBook": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        9,
                        "BookInput!"
                    ]
                }
            ],
            "deleteBook": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createBookTranslation": [
                11,
                {
                    "input": [
                        13,
                        "BookTranslationInput!"
                    ]
                }
            ],
            "updateBookTranslation": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        13,
                        "BookTranslationInput!"
                    ]
                }
            ],
            "deleteBookTranslation": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createBookCategory": [
                4,
                {
                    "input": [
                        6,
                        "BookCategoryInput!"
                    ]
                }
            ],
            "updateBookCategory": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        6,
                        "BookCategoryInput!"
                    ]
                }
            ],
            "deleteBookCategory": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createChapter": [
                21,
                {
                    "input": [
                        23,
                        "ChapterInput!"
                    ]
                }
            ],
            "updateChapter": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        23,
                        "ChapterInput!"
                    ]
                }
            ],
            "deleteChapter": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createVerse": [
                91,
                {
                    "input": [
                        101,
                        "VerseInput!"
                    ]
                }
            ],
            "updateVerse": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        101,
                        "VerseInput!"
                    ]
                }
            ],
            "deleteVerse": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createVerseContent": [
                92,
                {
                    "input": [
                        94,
                        "VerseContentInput!"
                    ]
                }
            ],
            "updateVerseContent": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        94,
                        "VerseContentInput!"
                    ]
                }
            ],
            "deleteVerseContent": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createLemmas": [
                55,
                {
                    "input": [
                        57,
                        "LemmasInput!"
                    ]
                }
            ],
            "updateLemmas": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        57,
                        "LemmasInput!"
                    ]
                }
            ],
            "deleteLemmas": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createTokenMorphology": [
                79,
                {
                    "input": [
                        81,
                        "TokenMorphologyInput!"
                    ]
                }
            ],
            "updateTokenMorphology": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        81,
                        "TokenMorphologyInput!"
                    ]
                }
            ],
            "deleteTokenMorphology": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createWordnetLinks": [
                116,
                {
                    "input": [
                        118,
                        "WordnetLinksInput!"
                    ]
                }
            ],
            "updateWordnetLinks": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        118,
                        "WordnetLinksInput!"
                    ]
                }
            ],
            "deleteWordnetLinks": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createVerseEmbeddings": [
                96,
                {
                    "input": [
                        98,
                        "VerseEmbeddingsInput!"
                    ]
                }
            ],
            "updateVerseEmbeddings": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        98,
                        "VerseEmbeddingsInput!"
                    ]
                }
            ],
            "deleteVerseEmbeddings": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createLexiconEmbeddings": [
                59,
                {
                    "input": [
                        61,
                        "LexiconEmbeddingsInput!"
                    ]
                }
            ],
            "updateLexiconEmbeddings": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        61,
                        "LexiconEmbeddingsInput!"
                    ]
                }
            ],
            "deleteLexiconEmbeddings": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createSummaries": [
                71,
                {
                    "input": [
                        73,
                        "SummariesInput!"
                    ]
                }
            ],
            "updateSummaries": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        73,
                        "SummariesInput!"
                    ]
                }
            ],
            "deleteSummaries": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createSyntaxRelations": [
                75,
                {
                    "input": [
                        77,
                        "SyntaxRelationsInput!"
                    ]
                }
            ],
            "updateSyntaxRelations": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        77,
                        "SyntaxRelationsInput!"
                    ]
                }
            ],
            "deleteSyntaxRelations": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createInterlinearAlignments": [
                41,
                {
                    "input": [
                        43,
                        "InterlinearAlignmentsInput!"
                    ]
                }
            ],
            "updateInterlinearAlignments": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        43,
                        "InterlinearAlignmentsInput!"
                    ]
                }
            ],
            "deleteInterlinearAlignments": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createEntities": [
                27,
                {
                    "input": [
                        29,
                        "EntitiesInput!"
                    ]
                }
            ],
            "updateEntities": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        29,
                        "EntitiesInput!"
                    ]
                }
            ],
            "deleteEntities": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createEntityMentions": [
                36,
                {
                    "input": [
                        38,
                        "EntityMentionsInput!"
                    ]
                }
            ],
            "updateEntityMentions": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        38,
                        "EntityMentionsInput!"
                    ]
                }
            ],
            "deleteEntityMentions": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createEntityEdges": [
                31,
                {
                    "input": [
                        34,
                        "EntityEdgesInput!"
                    ]
                }
            ],
            "updateEntityEdges": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        34,
                        "EntityEdgesInput!"
                    ]
                }
            ],
            "deleteEntityEdges": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createUnifiedBible": [
                87,
                {
                    "input": [
                        89,
                        "UnifiedBibleInput!"
                    ]
                }
            ],
            "updateUnifiedBible": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        89,
                        "UnifiedBibleInput!"
                    ]
                }
            ],
            "deleteUnifiedBible": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createVideo": [
                103,
                {
                    "input": [
                        105,
                        "VideoInput!"
                    ]
                }
            ],
            "updateVideo": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        105,
                        "VideoInput!"
                    ]
                }
            ],
            "deleteVideo": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createVideoView": [
                112,
                {
                    "input": [
                        114,
                        "VideoViewInput!"
                    ]
                }
            ],
            "updateVideoView": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        114,
                        "VideoViewInput!"
                    ]
                }
            ],
            "deleteVideoView": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createKeyword": [
                47,
                {
                    "input": [
                        49,
                        "KeywordInput!"
                    ]
                }
            ],
            "updateKeyword": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        49,
                        "KeywordInput!"
                    ]
                }
            ],
            "deleteKeyword": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "createVideoKeyword": [
                106,
                {
                    "input": [
                        108,
                        "VideoKeywordInput!"
                    ]
                }
            ],
            "updateVideoKeyword": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ],
                    "input": [
                        108,
                        "VideoKeywordInput!"
                    ]
                }
            ],
            "deleteVideoKeyword": [
                16,
                {
                    "uid": [
                        1,
                        "ID!"
                    ]
                }
            ],
            "flushDatabase": [
                16
            ],
            "compactDatabase": [
                3
            ],
            "__typename": [
                2
            ]
        },
        "MutationEvent": {
            "type": [
                2
            ],
            "uid": [
                1
            ],
            "mutation": [
                65
            ],
            "payload": [
                46
            ],
            "__typename": [
                2
            ]
        },
        "MutationType": {},
        "Query": {
            "queryLanguage": [
                51,
                {
                    "filter": [
                        52
                    ],
                    "sort": [
                        54
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getLanguage": [
                51,
                {
                    "uid": [
                        1
                    ],
                    "code": [
                        2
                    ]
                }
            ],
            "queryCategory": [
                17,
                {
                    "filter": [
                        18
                    ],
                    "sort": [
                        20
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getCategory": [
                17,
                {
                    "uid": [
                        1
                    ],
                    "name": [
                        2
                    ]
                }
            ],
            "queryTranslations": [
                83,
                {
                    "filter": [
                        84
                    ],
                    "sort": [
                        86
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getTranslations": [
                83,
                {
                    "uid": [
                        1
                    ],
                    "code": [
                        2
                    ]
                }
            ],
            "queryBook": [
                0,
                {
                    "filter": [
                        8
                    ],
                    "sort": [
                        10
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getBook": [
                0,
                {
                    "uid": [
                        1
                    ],
                    "code": [
                        2
                    ]
                }
            ],
            "queryBookTranslation": [
                11,
                {
                    "filter": [
                        12
                    ],
                    "sort": [
                        14
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getBookTranslation": [
                11,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryBookCategory": [
                4,
                {
                    "filter": [
                        5
                    ],
                    "sort": [
                        7
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getBookCategory": [
                4,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryChapter": [
                21,
                {
                    "filter": [
                        22
                    ],
                    "sort": [
                        24
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getChapter": [
                21,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryVerse": [
                91,
                {
                    "filter": [
                        100
                    ],
                    "sort": [
                        102
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getVerse": [
                91,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryVerseContent": [
                92,
                {
                    "filter": [
                        93
                    ],
                    "sort": [
                        95
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getVerseContent": [
                92,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryLemmas": [
                55,
                {
                    "filter": [
                        56
                    ],
                    "sort": [
                        58
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getLemmas": [
                55,
                {
                    "uid": [
                        1
                    ],
                    "lemmaId": [
                        2
                    ]
                }
            ],
            "queryTokenMorphology": [
                79,
                {
                    "filter": [
                        80
                    ],
                    "sort": [
                        82
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getTokenMorphology": [
                79,
                {
                    "uid": [
                        1
                    ],
                    "morphId": [
                        2
                    ]
                }
            ],
            "queryWordnetLinks": [
                116,
                {
                    "filter": [
                        117
                    ],
                    "sort": [
                        119
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getWordnetLinks": [
                116,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryVerseEmbeddings": [
                96,
                {
                    "filter": [
                        97
                    ],
                    "sort": [
                        99
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getVerseEmbeddings": [
                96,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryLexiconEmbeddings": [
                59,
                {
                    "filter": [
                        60
                    ],
                    "sort": [
                        62
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getLexiconEmbeddings": [
                59,
                {
                    "uid": [
                        1
                    ],
                    "embeddingId": [
                        2
                    ]
                }
            ],
            "querySummaries": [
                71,
                {
                    "filter": [
                        72
                    ],
                    "sort": [
                        74
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getSummaries": [
                71,
                {
                    "uid": [
                        1
                    ],
                    "summaryId": [
                        2
                    ]
                }
            ],
            "querySyntaxRelations": [
                75,
                {
                    "filter": [
                        76
                    ],
                    "sort": [
                        78
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getSyntaxRelations": [
                75,
                {
                    "uid": [
                        1
                    ],
                    "relationId": [
                        2
                    ]
                }
            ],
            "queryInterlinearAlignments": [
                41,
                {
                    "filter": [
                        42
                    ],
                    "sort": [
                        44
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getInterlinearAlignments": [
                41,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryEntities": [
                27,
                {
                    "filter": [
                        28
                    ],
                    "sort": [
                        30
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getEntities": [
                27,
                {
                    "uid": [
                        1
                    ],
                    "entityId": [
                        2
                    ]
                }
            ],
            "queryEntityMentions": [
                36,
                {
                    "filter": [
                        37
                    ],
                    "sort": [
                        39
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getEntityMentions": [
                36,
                {
                    "uid": [
                        1
                    ],
                    "mentionId": [
                        2
                    ]
                }
            ],
            "queryEntityEdges": [
                31,
                {
                    "filter": [
                        33
                    ],
                    "sort": [
                        35
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getEntityEdges": [
                31,
                {
                    "uid": [
                        1
                    ],
                    "edgeId": [
                        2
                    ]
                }
            ],
            "queryUnifiedBible": [
                87,
                {
                    "filter": [
                        88
                    ],
                    "sort": [
                        90
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getUnifiedBible": [
                87,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryVideo": [
                103,
                {
                    "filter": [
                        104
                    ],
                    "sort": [
                        110
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getVideo": [
                103,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryVideoView": [
                112,
                {
                    "filter": [
                        113
                    ],
                    "sort": [
                        115
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getVideoView": [
                112,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "queryKeyword": [
                47,
                {
                    "filter": [
                        48
                    ],
                    "sort": [
                        50
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getKeyword": [
                47,
                {
                    "uid": [
                        1
                    ],
                    "text": [
                        2
                    ]
                }
            ],
            "queryVideoKeyword": [
                106,
                {
                    "filter": [
                        107
                    ],
                    "sort": [
                        109
                    ],
                    "first": [
                        3
                    ],
                    "after": [
                        2
                    ]
                }
            ],
            "getVideoKeyword": [
                106,
                {
                    "uid": [
                        1
                    ]
                }
            ],
            "search": [
                67,
                {
                    "vector": [
                        32,
                        "[Float!]"
                    ],
                    "k": [
                        3
                    ]
                }
            ],
            "hybridSearch": [
                67,
                {
                    "vector": [
                        32,
                        "[Float!]"
                    ],
                    "text": [
                        2,
                        "String!"
                    ],
                    "field": [
                        2,
                        "String!"
                    ],
                    "k": [
                        3
                    ]
                }
            ],
            "__typename": [
                2
            ]
        },
        "SearchResult": {
            "uid": [
                1
            ],
            "distance": [
                32
            ],
            "__typename": [
                2
            ]
        },
        "SortDirection": {},
        "StringFilter": {
            "eq": [
                2
            ],
            "contains": [
                2
            ],
            "allofterms": [
                2
            ],
            "anyofterms": [
                2
            ],
            "alloftext": [
                2
            ],
            "anyoftext": [
                2
            ],
            "lt": [
                2
            ],
            "le": [
                2
            ],
            "gt": [
                2
            ],
            "ge": [
                2
            ],
            "in": [
                2
            ],
            "__typename": [
                2
            ]
        },
        "Subscription": {
            "event": [
                64,
                {
                    "types": [
                        2,
                        "[String]"
                    ]
                }
            ],
            "__typename": [
                2
            ]
        },
        "Summaries": {
            "uid": [
                1
            ],
            "summaryId": [
                3
            ],
            "summaryText": [
                2
            ],
            "level": [
                3
            ],
            "book": [
                0
            ],
            "chapter": [
                21
            ],
            "__typename": [
                2
            ]
        },
        "SummariesFilter": {
            "summaryId": [
                45
            ],
            "summaryText": [
                69
            ],
            "level": [
                45
            ],
            "and": [
                72
            ],
            "or": [
                72
            ],
            "not": [
                72
            ],
            "__typename": [
                2
            ]
        },
        "SummariesInput": {
            "uid": [
                1
            ],
            "summaryId": [
                3
            ],
            "summaryText": [
                2
            ],
            "level": [
                3
            ],
            "book": [
                9
            ],
            "chapter": [
                23
            ],
            "__typename": [
                2
            ]
        },
        "SummariesSort": {
            "summaryId": [
                68
            ],
            "summaryText": [
                68
            ],
            "level": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "SyntaxRelations": {
            "uid": [
                1
            ],
            "relationId": [
                3
            ],
            "subjectText": [
                2
            ],
            "verbText": [
                2
            ],
            "objectText": [
                2
            ],
            "relationType": [
                2
            ],
            "verse": [
                91
            ],
            "subjectLemma": [
                55
            ],
            "verbLemma": [
                55
            ],
            "objectLemma": [
                55
            ],
            "__typename": [
                2
            ]
        },
        "SyntaxRelationsFilter": {
            "relationId": [
                45
            ],
            "subjectText": [
                69
            ],
            "verbText": [
                69
            ],
            "objectText": [
                69
            ],
            "relationType": [
                69
            ],
            "and": [
                76
            ],
            "or": [
                76
            ],
            "not": [
                76
            ],
            "__typename": [
                2
            ]
        },
        "SyntaxRelationsInput": {
            "uid": [
                1
            ],
            "relationId": [
                3
            ],
            "subjectText": [
                2
            ],
            "verbText": [
                2
            ],
            "objectText": [
                2
            ],
            "relationType": [
                2
            ],
            "verse": [
                101
            ],
            "subjectLemma": [
                57
            ],
            "verbLemma": [
                57
            ],
            "objectLemma": [
                57
            ],
            "__typename": [
                2
            ]
        },
        "SyntaxRelationsSort": {
            "relationId": [
                68
            ],
            "subjectText": [
                68
            ],
            "verbText": [
                68
            ],
            "objectText": [
                68
            ],
            "relationType": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "TokenMorphology": {
            "uid": [
                1
            ],
            "morphId": [
                3
            ],
            "partOfSpeech": [
                2
            ],
            "person": [
                2
            ],
            "number": [
                2
            ],
            "gender": [
                2
            ],
            "tense": [
                2
            ],
            "mood": [
                2
            ],
            "voice": [
                2
            ],
            "caseVal": [
                2
            ],
            "verseContent": [
                92
            ],
            "__typename": [
                2
            ]
        },
        "TokenMorphologyFilter": {
            "morphId": [
                45
            ],
            "partOfSpeech": [
                69
            ],
            "person": [
                69
            ],
            "number": [
                69
            ],
            "gender": [
                69
            ],
            "tense": [
                69
            ],
            "mood": [
                69
            ],
            "voice": [
                69
            ],
            "caseVal": [
                69
            ],
            "and": [
                80
            ],
            "or": [
                80
            ],
            "not": [
                80
            ],
            "__typename": [
                2
            ]
        },
        "TokenMorphologyInput": {
            "uid": [
                1
            ],
            "morphId": [
                3
            ],
            "partOfSpeech": [
                2
            ],
            "person": [
                2
            ],
            "number": [
                2
            ],
            "gender": [
                2
            ],
            "tense": [
                2
            ],
            "mood": [
                2
            ],
            "voice": [
                2
            ],
            "caseVal": [
                2
            ],
            "verseContent": [
                94
            ],
            "__typename": [
                2
            ]
        },
        "TokenMorphologySort": {
            "morphId": [
                68
            ],
            "partOfSpeech": [
                68
            ],
            "person": [
                68
            ],
            "number": [
                68
            ],
            "gender": [
                68
            ],
            "tense": [
                68
            ],
            "mood": [
                68
            ],
            "voice": [
                68
            ],
            "caseVal": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "Translations": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "code": [
                2
            ],
            "name": [
                2
            ],
            "isInterlinear": [
                16
            ],
            "language": [
                51
            ],
            "bookTranslations": [
                11
            ],
            "verseContents": [
                92
            ],
            "__typename": [
                2
            ]
        },
        "TranslationsFilter": {
            "code": [
                69
            ],
            "name": [
                69
            ],
            "isInterlinear": [
                15
            ],
            "and": [
                84
            ],
            "or": [
                84
            ],
            "not": [
                84
            ],
            "__typename": [
                2
            ]
        },
        "TranslationsInput": {
            "uid": [
                1
            ],
            "code": [
                2
            ],
            "name": [
                2
            ],
            "isInterlinear": [
                16
            ],
            "language": [
                53
            ],
            "bookTranslations": [
                13
            ],
            "verseContents": [
                94
            ],
            "__typename": [
                2
            ]
        },
        "TranslationsSort": {
            "code": [
                68
            ],
            "name": [
                68
            ],
            "isInterlinear": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "UnifiedBible": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "verseNumber": [
                3
            ],
            "chapter": [
                3
            ],
            "text": [
                2
            ],
            "gospel": [
                2
            ],
            "sourceVerse": [
                2
            ],
            "notes": [
                2
            ],
            "chapterHeading": [
                2
            ],
            "sectionHeading": [
                2
            ],
            "metadata": [
                2
            ],
            "__typename": [
                2
            ]
        },
        "UnifiedBibleFilter": {
            "verseNumber": [
                45
            ],
            "chapter": [
                45
            ],
            "text": [
                69
            ],
            "gospel": [
                69
            ],
            "sourceVerse": [
                69
            ],
            "notes": [
                69
            ],
            "chapterHeading": [
                69
            ],
            "sectionHeading": [
                69
            ],
            "metadata": [
                69
            ],
            "and": [
                88
            ],
            "or": [
                88
            ],
            "not": [
                88
            ],
            "__typename": [
                2
            ]
        },
        "UnifiedBibleInput": {
            "uid": [
                1
            ],
            "verseNumber": [
                3
            ],
            "chapter": [
                3
            ],
            "text": [
                2
            ],
            "gospel": [
                2
            ],
            "sourceVerse": [
                2
            ],
            "notes": [
                2
            ],
            "chapterHeading": [
                2
            ],
            "sectionHeading": [
                2
            ],
            "metadata": [
                2
            ],
            "__typename": [
                2
            ]
        },
        "UnifiedBibleSort": {
            "verseNumber": [
                68
            ],
            "chapter": [
                68
            ],
            "text": [
                68
            ],
            "gospel": [
                68
            ],
            "sourceVerse": [
                68
            ],
            "notes": [
                68
            ],
            "chapterHeading": [
                68
            ],
            "sectionHeading": [
                68
            ],
            "metadata": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "Verse": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "number": [
                3
            ],
            "chunkType": [
                2
            ],
            "chapterHeading": [
                2
            ],
            "sectionHeading": [
                2
            ],
            "paragraphStart": [
                16
            ],
            "poetryIndent": [
                3
            ],
            "metadata": [
                2
            ],
            "chapter": [
                21
            ],
            "verseContents": [
                92,
                {
                    "filter": [
                        93
                    ]
                }
            ],
            "verseEmbeddings": [
                96
            ],
            "syntaxRelations": [
                75
            ],
            "entityMentions": [
                36
            ],
            "__typename": [
                2
            ]
        },
        "VerseContent": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "type": [
                2
            ],
            "text": [
                2
            ],
            "lemma": [
                2
            ],
            "strong": [
                2
            ],
            "morph": [
                2
            ],
            "gloss": [
                2
            ],
            "verse": [
                91
            ],
            "translation": [
                83
            ],
            "tokenMorphologies": [
                79
            ],
            "__typename": [
                2
            ]
        },
        "VerseContentFilter": {
            "type": [
                69
            ],
            "text": [
                69
            ],
            "lemma": [
                69
            ],
            "strong": [
                69
            ],
            "morph": [
                69
            ],
            "gloss": [
                69
            ],
            "and": [
                93
            ],
            "or": [
                93
            ],
            "not": [
                93
            ],
            "__typename": [
                2
            ]
        },
        "VerseContentInput": {
            "uid": [
                1
            ],
            "type": [
                2
            ],
            "text": [
                2
            ],
            "lemma": [
                2
            ],
            "strong": [
                2
            ],
            "morph": [
                2
            ],
            "gloss": [
                2
            ],
            "verse": [
                101
            ],
            "translation": [
                85
            ],
            "tokenMorphologies": [
                81
            ],
            "__typename": [
                2
            ]
        },
        "VerseContentSort": {
            "type": [
                68
            ],
            "text": [
                68
            ],
            "lemma": [
                68
            ],
            "strong": [
                68
            ],
            "morph": [
                68
            ],
            "gloss": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "VerseEmbeddings": {
            "uid": [
                1
            ],
            "embeddingId": [
                1
            ],
            "modelName": [
                2
            ],
            "verse": [
                91
            ],
            "__typename": [
                2
            ]
        },
        "VerseEmbeddingsFilter": {
            "modelName": [
                69
            ],
            "and": [
                97
            ],
            "or": [
                97
            ],
            "not": [
                97
            ],
            "__typename": [
                2
            ]
        },
        "VerseEmbeddingsInput": {
            "uid": [
                1
            ],
            "modelName": [
                2
            ],
            "verse": [
                101
            ],
            "__typename": [
                2
            ]
        },
        "VerseEmbeddingsSort": {
            "modelName": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "VerseFilter": {
            "number": [
                45
            ],
            "chunkType": [
                69
            ],
            "chapterHeading": [
                69
            ],
            "sectionHeading": [
                69
            ],
            "paragraphStart": [
                15
            ],
            "poetryIndent": [
                45
            ],
            "metadata": [
                69
            ],
            "and": [
                100
            ],
            "or": [
                100
            ],
            "not": [
                100
            ],
            "__typename": [
                2
            ]
        },
        "VerseInput": {
            "uid": [
                1
            ],
            "number": [
                3
            ],
            "chunkType": [
                2
            ],
            "chapterHeading": [
                2
            ],
            "sectionHeading": [
                2
            ],
            "paragraphStart": [
                16
            ],
            "poetryIndent": [
                3
            ],
            "metadata": [
                2
            ],
            "chapter": [
                23
            ],
            "verseContents": [
                94
            ],
            "verseEmbeddings": [
                98
            ],
            "syntaxRelations": [
                77
            ],
            "entityMentions": [
                38
            ],
            "__typename": [
                2
            ]
        },
        "VerseSort": {
            "number": [
                68
            ],
            "chunkType": [
                68
            ],
            "chapterHeading": [
                68
            ],
            "sectionHeading": [
                68
            ],
            "paragraphStart": [
                68
            ],
            "poetryIndent": [
                68
            ],
            "metadata": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "Video": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "name": [
                2
            ],
            "status": [
                111
            ],
            "createdAt": [
                25
            ],
            "updatedAt": [
                25
            ],
            "script": [
                2
            ],
            "generatedFilename": [
                2
            ],
            "generatedTitle": [
                2
            ],
            "generatedDescription": [
                2
            ],
            "generatedTags": [
                2
            ],
            "thumbnailPromptA": [
                2
            ],
            "thumbnailPromptB": [
                2
            ],
            "checklist": [
                2
            ],
            "views": [
                112
            ],
            "keywords": [
                106
            ],
            "__typename": [
                2
            ]
        },
        "VideoFilter": {
            "name": [
                69
            ],
            "status": [
                69
            ],
            "createdAt": [
                26
            ],
            "updatedAt": [
                26
            ],
            "script": [
                69
            ],
            "generatedFilename": [
                69
            ],
            "generatedTitle": [
                69
            ],
            "generatedDescription": [
                69
            ],
            "generatedTags": [
                69
            ],
            "thumbnailPromptA": [
                69
            ],
            "thumbnailPromptB": [
                69
            ],
            "checklist": [
                69
            ],
            "and": [
                104
            ],
            "or": [
                104
            ],
            "not": [
                104
            ],
            "__typename": [
                2
            ]
        },
        "VideoInput": {
            "uid": [
                1
            ],
            "name": [
                2
            ],
            "status": [
                111
            ],
            "createdAt": [
                25
            ],
            "updatedAt": [
                25
            ],
            "script": [
                2
            ],
            "generatedFilename": [
                2
            ],
            "generatedTitle": [
                2
            ],
            "generatedDescription": [
                2
            ],
            "generatedTags": [
                2
            ],
            "thumbnailPromptA": [
                2
            ],
            "thumbnailPromptB": [
                2
            ],
            "checklist": [
                2
            ],
            "views": [
                114
            ],
            "keywords": [
                108
            ],
            "__typename": [
                2
            ]
        },
        "VideoKeyword": {
            "uid": [
                1
            ],
            "video": [
                103
            ],
            "keyword": [
                47
            ],
            "__typename": [
                2
            ]
        },
        "VideoKeywordFilter": {
            "and": [
                107
            ],
            "or": [
                107
            ],
            "not": [
                107
            ],
            "__typename": [
                2
            ]
        },
        "VideoKeywordInput": {
            "uid": [
                1
            ],
            "video": [
                105
            ],
            "keyword": [
                49
            ],
            "__typename": [
                2
            ]
        },
        "VideoKeywordSort": {
            "id": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "VideoSort": {
            "name": [
                68
            ],
            "status": [
                68
            ],
            "createdAt": [
                68
            ],
            "updatedAt": [
                68
            ],
            "script": [
                68
            ],
            "generatedFilename": [
                68
            ],
            "generatedTitle": [
                68
            ],
            "generatedDescription": [
                68
            ],
            "generatedTags": [
                68
            ],
            "thumbnailPromptA": [
                68
            ],
            "thumbnailPromptB": [
                68
            ],
            "checklist": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "VideoStatus": {},
        "VideoView": {
            "uid": [
                1
            ],
            "id": [
                1
            ],
            "date": [
                25
            ],
            "count": [
                3
            ],
            "video": [
                103
            ],
            "__typename": [
                2
            ]
        },
        "VideoViewFilter": {
            "date": [
                26
            ],
            "count": [
                45
            ],
            "and": [
                113
            ],
            "or": [
                113
            ],
            "not": [
                113
            ],
            "__typename": [
                2
            ]
        },
        "VideoViewInput": {
            "uid": [
                1
            ],
            "date": [
                25
            ],
            "count": [
                3
            ],
            "video": [
                105
            ],
            "__typename": [
                2
            ]
        },
        "VideoViewSort": {
            "date": [
                68
            ],
            "count": [
                68
            ],
            "__typename": [
                2
            ]
        },
        "WordnetLinks": {
            "uid": [
                1
            ],
            "synsetId": [
                2
            ],
            "similarityScore": [
                32
            ],
            "lemma": [
                55
            ],
            "__typename": [
                2
            ]
        },
        "WordnetLinksFilter": {
            "synsetId": [
                69
            ],
            "similarityScore": [
                40
            ],
            "and": [
                117
            ],
            "or": [
                117
            ],
            "not": [
                117
            ],
            "__typename": [
                2
            ]
        },
        "WordnetLinksInput": {
            "uid": [
                1
            ],
            "synsetId": [
                2
            ],
            "similarityScore": [
                32
            ],
            "lemma": [
                57
            ],
            "__typename": [
                2
            ]
        },
        "WordnetLinksSort": {
            "synsetId": [
                68
            ],
            "similarityScore": [
                68
            ],
            "__typename": [
                2
            ]
        }
    }
}