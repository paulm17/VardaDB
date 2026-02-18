// @ts-nocheck
/* istanbul ignore file */
/* tslint:disable */
/* eslint-disable */

export type Scalars = {
    ID: string,
    String: string,
    Int: number,
    Boolean: boolean,
    DateTime: any,
    Float: number,
    JSON: any,
}

export interface Book {
    uid: Scalars['ID']
    id: Scalars['ID']
    code: (Scalars['String'] | null)
    nameEn: (Scalars['String'] | null)
    nameHe: (Scalars['String'] | null)
    nameGr: (Scalars['String'] | null)
    chapters: (Scalars['Int'] | null)
    bookTranslations: ((BookTranslation | null)[] | null)
    bookCategories: ((BookCategory | null)[] | null)
    summaries: ((Summaries | null)[] | null)
    __typename: 'Book'
}

export interface BookCategory {
    uid: Scalars['ID']
    id: Scalars['ID']
    book: (Book | null)
    category: (Category | null)
    __typename: 'BookCategory'
}

export interface BookTranslation {
    uid: Scalars['ID']
    id: Scalars['ID']
    book: (Book | null)
    translation: (Translations | null)
    chapters: ((Chapter | null)[] | null)
    __typename: 'BookTranslation'
}

export interface Category {
    uid: Scalars['ID']
    id: Scalars['ID']
    name: (Scalars['String'] | null)
    bookCategories: ((BookCategory | null)[] | null)
    __typename: 'Category'
}

export interface Chapter {
    uid: Scalars['ID']
    id: Scalars['ID']
    number: (Scalars['Int'] | null)
    bookTranslation: (BookTranslation | null)
    verses: ((Verse | null)[] | null)
    summaries: ((Summaries | null)[] | null)
    __typename: 'Chapter'
}

export interface Entities {
    uid: Scalars['ID']
    entityId: (Scalars['Int'] | null)
    name: (Scalars['String'] | null)
    entityType: (Scalars['String'] | null)
    description: (Scalars['String'] | null)
    mentions: ((EntityMentions | null)[] | null)
    edgesA: ((EntityEdges | null)[] | null)
    edgesB: ((EntityEdges | null)[] | null)
    __typename: 'Entities'
}

export interface EntityEdges {
    uid: Scalars['ID']
    edgeId: (Scalars['Int'] | null)
    relationType: (Scalars['String'] | null)
    weight: (Scalars['Float'] | null)
    entityA: (Entities | null)
    entityB: (Entities | null)
    __typename: 'EntityEdges'
}

export interface EntityMentions {
    uid: Scalars['ID']
    mentionId: (Scalars['Int'] | null)
    tokenStart: (Scalars['Int'] | null)
    tokenEnd: (Scalars['Int'] | null)
    confidence: (Scalars['Float'] | null)
    entity: (Entities | null)
    verse: (Verse | null)
    __typename: 'EntityMentions'
}

export interface InterlinearAlignments {
    uid: Scalars['ID']
    id: Scalars['ID']
    lemmaA: (Lemmas | null)
    lemmaB: (Lemmas | null)
    __typename: 'InterlinearAlignments'
}

export interface Keyword {
    uid: Scalars['ID']
    id: Scalars['ID']
    text: (Scalars['String'] | null)
    videos: ((VideoKeyword | null)[] | null)
    __typename: 'Keyword'
}

export interface Language {
    uid: Scalars['ID']
    id: Scalars['ID']
    code: (Scalars['String'] | null)
    name: (Scalars['String'] | null)
    translations: ((Translations | null)[] | null)
    __typename: 'Language'
}

export interface Lemmas {
    uid: Scalars['ID']
    lemmaId: (Scalars['Int'] | null)
    strongsId: (Scalars['String'] | null)
    languageCode: (Scalars['String'] | null)
    lemmaText: (Scalars['String'] | null)
    gloss: (Scalars['String'] | null)
    wordnetLinks: ((WordnetLinks | null)[] | null)
    lexiconEmbeddings: ((LexiconEmbeddings | null)[] | null)
    syntaxRelationsSub: ((SyntaxRelations | null)[] | null)
    syntaxRelationsVerb: ((SyntaxRelations | null)[] | null)
    syntaxRelationsObj: ((SyntaxRelations | null)[] | null)
    interlinearAlignmentsA: ((InterlinearAlignments | null)[] | null)
    interlinearAlignmentsB: ((InterlinearAlignments | null)[] | null)
    __typename: 'Lemmas'
}

export interface LexiconEmbeddings {
    uid: Scalars['ID']
    embeddingId: (Scalars['Int'] | null)
    modelName: (Scalars['String'] | null)
    lemma: (Lemmas | null)
    __typename: 'LexiconEmbeddings'
}

export interface Mutation {
    createLanguage: (Language | null)
    updateLanguage: (Scalars['Boolean'] | null)
    deleteLanguage: (Scalars['Boolean'] | null)
    createCategory: (Category | null)
    updateCategory: (Scalars['Boolean'] | null)
    deleteCategory: (Scalars['Boolean'] | null)
    createTranslations: (Translations | null)
    updateTranslations: (Scalars['Boolean'] | null)
    deleteTranslations: (Scalars['Boolean'] | null)
    createBook: (Book | null)
    updateBook: (Scalars['Boolean'] | null)
    deleteBook: (Scalars['Boolean'] | null)
    createBookTranslation: (BookTranslation | null)
    updateBookTranslation: (Scalars['Boolean'] | null)
    deleteBookTranslation: (Scalars['Boolean'] | null)
    createBookCategory: (BookCategory | null)
    updateBookCategory: (Scalars['Boolean'] | null)
    deleteBookCategory: (Scalars['Boolean'] | null)
    createChapter: (Chapter | null)
    updateChapter: (Scalars['Boolean'] | null)
    deleteChapter: (Scalars['Boolean'] | null)
    createVerse: (Verse | null)
    updateVerse: (Scalars['Boolean'] | null)
    deleteVerse: (Scalars['Boolean'] | null)
    createVerseContent: (VerseContent | null)
    updateVerseContent: (Scalars['Boolean'] | null)
    deleteVerseContent: (Scalars['Boolean'] | null)
    createLemmas: (Lemmas | null)
    updateLemmas: (Scalars['Boolean'] | null)
    deleteLemmas: (Scalars['Boolean'] | null)
    createTokenMorphology: (TokenMorphology | null)
    updateTokenMorphology: (Scalars['Boolean'] | null)
    deleteTokenMorphology: (Scalars['Boolean'] | null)
    createWordnetLinks: (WordnetLinks | null)
    updateWordnetLinks: (Scalars['Boolean'] | null)
    deleteWordnetLinks: (Scalars['Boolean'] | null)
    createVerseEmbeddings: (VerseEmbeddings | null)
    updateVerseEmbeddings: (Scalars['Boolean'] | null)
    deleteVerseEmbeddings: (Scalars['Boolean'] | null)
    createLexiconEmbeddings: (LexiconEmbeddings | null)
    updateLexiconEmbeddings: (Scalars['Boolean'] | null)
    deleteLexiconEmbeddings: (Scalars['Boolean'] | null)
    createSummaries: (Summaries | null)
    updateSummaries: (Scalars['Boolean'] | null)
    deleteSummaries: (Scalars['Boolean'] | null)
    createSyntaxRelations: (SyntaxRelations | null)
    updateSyntaxRelations: (Scalars['Boolean'] | null)
    deleteSyntaxRelations: (Scalars['Boolean'] | null)
    createInterlinearAlignments: (InterlinearAlignments | null)
    updateInterlinearAlignments: (Scalars['Boolean'] | null)
    deleteInterlinearAlignments: (Scalars['Boolean'] | null)
    createEntities: (Entities | null)
    updateEntities: (Scalars['Boolean'] | null)
    deleteEntities: (Scalars['Boolean'] | null)
    createEntityMentions: (EntityMentions | null)
    updateEntityMentions: (Scalars['Boolean'] | null)
    deleteEntityMentions: (Scalars['Boolean'] | null)
    createEntityEdges: (EntityEdges | null)
    updateEntityEdges: (Scalars['Boolean'] | null)
    deleteEntityEdges: (Scalars['Boolean'] | null)
    createUnifiedBible: (UnifiedBible | null)
    updateUnifiedBible: (Scalars['Boolean'] | null)
    deleteUnifiedBible: (Scalars['Boolean'] | null)
    createVideo: (Video | null)
    updateVideo: (Scalars['Boolean'] | null)
    deleteVideo: (Scalars['Boolean'] | null)
    createVideoView: (VideoView | null)
    updateVideoView: (Scalars['Boolean'] | null)
    deleteVideoView: (Scalars['Boolean'] | null)
    createKeyword: (Keyword | null)
    updateKeyword: (Scalars['Boolean'] | null)
    deleteKeyword: (Scalars['Boolean'] | null)
    createVideoKeyword: (VideoKeyword | null)
    updateVideoKeyword: (Scalars['Boolean'] | null)
    deleteVideoKeyword: (Scalars['Boolean'] | null)
    flushDatabase: (Scalars['Boolean'] | null)
    compactDatabase: (Scalars['Int'] | null)
    __typename: 'Mutation'
}

export interface MutationEvent {
    type: Scalars['String']
    uid: Scalars['ID']
    mutation: MutationType
    payload: (Scalars['JSON'] | null)
    __typename: 'MutationEvent'
}

export type MutationType = 'CREATE' | 'UPDATE' | 'DELETE'

export interface Query {
    queryLanguage: ((Language | null)[] | null)
    getLanguage: (Language | null)
    queryCategory: ((Category | null)[] | null)
    getCategory: (Category | null)
    queryTranslations: ((Translations | null)[] | null)
    getTranslations: (Translations | null)
    queryBook: ((Book | null)[] | null)
    getBook: (Book | null)
    queryBookTranslation: ((BookTranslation | null)[] | null)
    getBookTranslation: (BookTranslation | null)
    queryBookCategory: ((BookCategory | null)[] | null)
    getBookCategory: (BookCategory | null)
    queryChapter: ((Chapter | null)[] | null)
    getChapter: (Chapter | null)
    queryVerse: ((Verse | null)[] | null)
    getVerse: (Verse | null)
    queryVerseContent: ((VerseContent | null)[] | null)
    getVerseContent: (VerseContent | null)
    queryLemmas: ((Lemmas | null)[] | null)
    getLemmas: (Lemmas | null)
    queryTokenMorphology: ((TokenMorphology | null)[] | null)
    getTokenMorphology: (TokenMorphology | null)
    queryWordnetLinks: ((WordnetLinks | null)[] | null)
    getWordnetLinks: (WordnetLinks | null)
    queryVerseEmbeddings: ((VerseEmbeddings | null)[] | null)
    getVerseEmbeddings: (VerseEmbeddings | null)
    queryLexiconEmbeddings: ((LexiconEmbeddings | null)[] | null)
    getLexiconEmbeddings: (LexiconEmbeddings | null)
    querySummaries: ((Summaries | null)[] | null)
    getSummaries: (Summaries | null)
    querySyntaxRelations: ((SyntaxRelations | null)[] | null)
    getSyntaxRelations: (SyntaxRelations | null)
    queryInterlinearAlignments: ((InterlinearAlignments | null)[] | null)
    getInterlinearAlignments: (InterlinearAlignments | null)
    queryEntities: ((Entities | null)[] | null)
    getEntities: (Entities | null)
    queryEntityMentions: ((EntityMentions | null)[] | null)
    getEntityMentions: (EntityMentions | null)
    queryEntityEdges: ((EntityEdges | null)[] | null)
    getEntityEdges: (EntityEdges | null)
    queryUnifiedBible: ((UnifiedBible | null)[] | null)
    getUnifiedBible: (UnifiedBible | null)
    queryVideo: ((Video | null)[] | null)
    getVideo: (Video | null)
    queryVideoView: ((VideoView | null)[] | null)
    getVideoView: (VideoView | null)
    queryKeyword: ((Keyword | null)[] | null)
    getKeyword: (Keyword | null)
    queryVideoKeyword: ((VideoKeyword | null)[] | null)
    getVideoKeyword: (VideoKeyword | null)
    search: (SearchResult[] | null)
    hybridSearch: (SearchResult[] | null)
    __typename: 'Query'
}

export interface SearchResult {
    uid: Scalars['ID']
    distance: Scalars['Float']
    __typename: 'SearchResult'
}

export type SortDirection = 'ASC' | 'DESC'

export interface Subscription {
    event: MutationEvent
    __typename: 'Subscription'
}

export interface Summaries {
    uid: Scalars['ID']
    summaryId: (Scalars['Int'] | null)
    summaryText: (Scalars['String'] | null)
    level: (Scalars['Int'] | null)
    book: (Book | null)
    chapter: (Chapter | null)
    __typename: 'Summaries'
}

export interface SyntaxRelations {
    uid: Scalars['ID']
    relationId: (Scalars['Int'] | null)
    subjectText: (Scalars['String'] | null)
    verbText: (Scalars['String'] | null)
    objectText: (Scalars['String'] | null)
    relationType: (Scalars['String'] | null)
    verse: (Verse | null)
    subjectLemma: (Lemmas | null)
    verbLemma: (Lemmas | null)
    objectLemma: (Lemmas | null)
    __typename: 'SyntaxRelations'
}

export interface TokenMorphology {
    uid: Scalars['ID']
    morphId: (Scalars['Int'] | null)
    partOfSpeech: (Scalars['String'] | null)
    person: (Scalars['String'] | null)
    number: (Scalars['String'] | null)
    gender: (Scalars['String'] | null)
    tense: (Scalars['String'] | null)
    mood: (Scalars['String'] | null)
    voice: (Scalars['String'] | null)
    caseVal: (Scalars['String'] | null)
    verseContent: (VerseContent | null)
    __typename: 'TokenMorphology'
}

export interface Translations {
    uid: Scalars['ID']
    id: Scalars['ID']
    code: (Scalars['String'] | null)
    name: (Scalars['String'] | null)
    isInterlinear: (Scalars['Boolean'] | null)
    language: (Language | null)
    bookTranslations: ((BookTranslation | null)[] | null)
    verseContents: ((VerseContent | null)[] | null)
    __typename: 'Translations'
}

export interface UnifiedBible {
    uid: Scalars['ID']
    id: Scalars['ID']
    verseNumber: (Scalars['Int'] | null)
    chapter: (Scalars['Int'] | null)
    text: (Scalars['String'] | null)
    gospel: (Scalars['String'] | null)
    sourceVerse: (Scalars['String'] | null)
    notes: (Scalars['String'] | null)
    chapterHeading: (Scalars['String'] | null)
    sectionHeading: (Scalars['String'] | null)
    metadata: (Scalars['String'] | null)
    __typename: 'UnifiedBible'
}

export interface Verse {
    uid: Scalars['ID']
    id: Scalars['ID']
    number: (Scalars['Int'] | null)
    chunkType: (Scalars['String'] | null)
    chapterHeading: (Scalars['String'] | null)
    sectionHeading: (Scalars['String'] | null)
    paragraphStart: (Scalars['Boolean'] | null)
    poetryIndent: (Scalars['Int'] | null)
    metadata: (Scalars['String'] | null)
    chapter: (Chapter | null)
    verseContents: ((VerseContent | null)[] | null)
    verseEmbeddings: ((VerseEmbeddings | null)[] | null)
    syntaxRelations: ((SyntaxRelations | null)[] | null)
    entityMentions: ((EntityMentions | null)[] | null)
    __typename: 'Verse'
}

export interface VerseContent {
    uid: Scalars['ID']
    id: Scalars['ID']
    type: (Scalars['String'] | null)
    text: (Scalars['String'] | null)
    lemma: (Scalars['String'] | null)
    strong: (Scalars['String'] | null)
    morph: (Scalars['String'] | null)
    gloss: (Scalars['String'] | null)
    verse: (Verse | null)
    translation: (Translations | null)
    tokenMorphologies: ((TokenMorphology | null)[] | null)
    __typename: 'VerseContent'
}

export interface VerseEmbeddings {
    uid: Scalars['ID']
    embeddingId: Scalars['ID']
    modelName: (Scalars['String'] | null)
    verse: (Verse | null)
    __typename: 'VerseEmbeddings'
}

export interface Video {
    uid: Scalars['ID']
    id: Scalars['ID']
    name: (Scalars['String'] | null)
    status: (VideoStatus | null)
    createdAt: (Scalars['DateTime'] | null)
    updatedAt: (Scalars['DateTime'] | null)
    script: (Scalars['String'] | null)
    generatedFilename: (Scalars['String'] | null)
    generatedTitle: (Scalars['String'] | null)
    generatedDescription: (Scalars['String'] | null)
    generatedTags: (Scalars['String'] | null)
    thumbnailPromptA: (Scalars['String'] | null)
    thumbnailPromptB: (Scalars['String'] | null)
    checklist: (Scalars['String'] | null)
    views: ((VideoView | null)[] | null)
    keywords: ((VideoKeyword | null)[] | null)
    __typename: 'Video'
}

export interface VideoKeyword {
    uid: Scalars['ID']
    video: (Video | null)
    keyword: (Keyword | null)
    __typename: 'VideoKeyword'
}

export type VideoStatus = 'NEW' | 'IN_PROGRESS' | 'COMPLETED' | 'SCHEDULED' | 'PUBLISHED' | 'DELETED'

export interface VideoView {
    uid: Scalars['ID']
    id: Scalars['ID']
    date: (Scalars['DateTime'] | null)
    count: (Scalars['Int'] | null)
    video: (Video | null)
    __typename: 'VideoView'
}

export interface WordnetLinks {
    uid: Scalars['ID']
    synsetId: (Scalars['String'] | null)
    similarityScore: (Scalars['Float'] | null)
    lemma: (Lemmas | null)
    __typename: 'WordnetLinks'
}

export interface BookGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    code?: boolean | number
    nameEn?: boolean | number
    nameHe?: boolean | number
    nameGr?: boolean | number
    chapters?: boolean | number
    bookTranslations?: BookTranslationGenqlSelection
    bookCategories?: BookCategoryGenqlSelection
    summaries?: SummariesGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface BookCategoryGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    book?: BookGenqlSelection
    category?: CategoryGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface BookCategoryFilter {and?: ((BookCategoryFilter | null)[] | null),or?: ((BookCategoryFilter | null)[] | null),not?: (BookCategoryFilter | null)}

export interface BookCategoryInput {uid?: (Scalars['ID'] | null),book?: (BookInput | null),category?: (CategoryInput | null)}

export interface BookCategorySort {id?: (SortDirection | null)}

export interface BookFilter {code?: (StringFilter | null),nameEn?: (StringFilter | null),nameHe?: (StringFilter | null),nameGr?: (StringFilter | null),chapters?: (IntFilter | null),and?: ((BookFilter | null)[] | null),or?: ((BookFilter | null)[] | null),not?: (BookFilter | null)}

export interface BookInput {uid?: (Scalars['ID'] | null),code?: (Scalars['String'] | null),nameEn?: (Scalars['String'] | null),nameHe?: (Scalars['String'] | null),nameGr?: (Scalars['String'] | null),chapters?: (Scalars['Int'] | null),bookTranslations?: ((BookTranslationInput | null)[] | null),bookCategories?: ((BookCategoryInput | null)[] | null),summaries?: ((SummariesInput | null)[] | null)}

export interface BookSort {code?: (SortDirection | null),nameEn?: (SortDirection | null),nameHe?: (SortDirection | null),nameGr?: (SortDirection | null),chapters?: (SortDirection | null)}

export interface BookTranslationGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    book?: BookGenqlSelection
    translation?: TranslationsGenqlSelection
    chapters?: (ChapterGenqlSelection & { __args?: {sort?: (ChapterSort | null)} })
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface BookTranslationFilter {and?: ((BookTranslationFilter | null)[] | null),or?: ((BookTranslationFilter | null)[] | null),not?: (BookTranslationFilter | null)}

export interface BookTranslationInput {uid?: (Scalars['ID'] | null),book?: (BookInput | null),translation?: (TranslationsInput | null),chapters?: ((ChapterInput | null)[] | null)}

export interface BookTranslationSort {id?: (SortDirection | null)}

export interface BooleanFilter {eq?: (Scalars['Boolean'] | null)}

export interface CategoryGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    name?: boolean | number
    bookCategories?: BookCategoryGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface CategoryFilter {name?: (StringFilter | null),and?: ((CategoryFilter | null)[] | null),or?: ((CategoryFilter | null)[] | null),not?: (CategoryFilter | null)}

export interface CategoryInput {uid?: (Scalars['ID'] | null),name?: (Scalars['String'] | null),bookCategories?: ((BookCategoryInput | null)[] | null)}

export interface CategorySort {name?: (SortDirection | null)}

export interface ChapterGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    number?: boolean | number
    bookTranslation?: BookTranslationGenqlSelection
    verses?: (VerseGenqlSelection & { __args?: {filter?: (VerseFilter | null)} })
    summaries?: SummariesGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface ChapterFilter {number?: (IntFilter | null),and?: ((ChapterFilter | null)[] | null),or?: ((ChapterFilter | null)[] | null),not?: (ChapterFilter | null)}

export interface ChapterInput {uid?: (Scalars['ID'] | null),number?: (Scalars['Int'] | null),bookTranslation?: (BookTranslationInput | null),verses?: ((VerseInput | null)[] | null),summaries?: ((SummariesInput | null)[] | null)}

export interface ChapterSort {number?: (SortDirection | null)}

export interface DateTimeFilter {eq?: (Scalars['DateTime'] | null),gt?: (Scalars['DateTime'] | null),lt?: (Scalars['DateTime'] | null),ge?: (Scalars['DateTime'] | null),le?: (Scalars['DateTime'] | null),in?: ((Scalars['DateTime'] | null)[] | null)}

export interface EntitiesGenqlSelection{
    uid?: boolean | number
    entityId?: boolean | number
    name?: boolean | number
    entityType?: boolean | number
    description?: boolean | number
    mentions?: EntityMentionsGenqlSelection
    edgesA?: EntityEdgesGenqlSelection
    edgesB?: EntityEdgesGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface EntitiesFilter {entityId?: (IntFilter | null),name?: (StringFilter | null),entityType?: (StringFilter | null),description?: (StringFilter | null),and?: ((EntitiesFilter | null)[] | null),or?: ((EntitiesFilter | null)[] | null),not?: (EntitiesFilter | null)}

export interface EntitiesInput {uid?: (Scalars['ID'] | null),entityId?: (Scalars['Int'] | null),name?: (Scalars['String'] | null),entityType?: (Scalars['String'] | null),description?: (Scalars['String'] | null),mentions?: ((EntityMentionsInput | null)[] | null),edgesA?: ((EntityEdgesInput | null)[] | null),edgesB?: ((EntityEdgesInput | null)[] | null)}

export interface EntitiesSort {entityId?: (SortDirection | null),name?: (SortDirection | null),entityType?: (SortDirection | null),description?: (SortDirection | null)}

export interface EntityEdgesGenqlSelection{
    uid?: boolean | number
    edgeId?: boolean | number
    relationType?: boolean | number
    weight?: boolean | number
    entityA?: EntitiesGenqlSelection
    entityB?: EntitiesGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface EntityEdgesFilter {edgeId?: (IntFilter | null),relationType?: (StringFilter | null),weight?: (FloatFilter | null),and?: ((EntityEdgesFilter | null)[] | null),or?: ((EntityEdgesFilter | null)[] | null),not?: (EntityEdgesFilter | null)}

export interface EntityEdgesInput {uid?: (Scalars['ID'] | null),edgeId?: (Scalars['Int'] | null),relationType?: (Scalars['String'] | null),weight?: (Scalars['Float'] | null),entityA?: (EntitiesInput | null),entityB?: (EntitiesInput | null)}

export interface EntityEdgesSort {edgeId?: (SortDirection | null),relationType?: (SortDirection | null),weight?: (SortDirection | null)}

export interface EntityMentionsGenqlSelection{
    uid?: boolean | number
    mentionId?: boolean | number
    tokenStart?: boolean | number
    tokenEnd?: boolean | number
    confidence?: boolean | number
    entity?: EntitiesGenqlSelection
    verse?: VerseGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface EntityMentionsFilter {mentionId?: (IntFilter | null),tokenStart?: (IntFilter | null),tokenEnd?: (IntFilter | null),confidence?: (FloatFilter | null),and?: ((EntityMentionsFilter | null)[] | null),or?: ((EntityMentionsFilter | null)[] | null),not?: (EntityMentionsFilter | null)}

export interface EntityMentionsInput {uid?: (Scalars['ID'] | null),mentionId?: (Scalars['Int'] | null),tokenStart?: (Scalars['Int'] | null),tokenEnd?: (Scalars['Int'] | null),confidence?: (Scalars['Float'] | null),entity?: (EntitiesInput | null),verse?: (VerseInput | null)}

export interface EntityMentionsSort {mentionId?: (SortDirection | null),tokenStart?: (SortDirection | null),tokenEnd?: (SortDirection | null),confidence?: (SortDirection | null)}

export interface FloatFilter {eq?: (Scalars['Float'] | null),gt?: (Scalars['Float'] | null),lt?: (Scalars['Float'] | null),ge?: (Scalars['Float'] | null),le?: (Scalars['Float'] | null),between?: ((Scalars['Float'] | null)[] | null),in?: ((Scalars['Float'] | null)[] | null)}

export interface InterlinearAlignmentsGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    lemmaA?: LemmasGenqlSelection
    lemmaB?: LemmasGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface InterlinearAlignmentsFilter {and?: ((InterlinearAlignmentsFilter | null)[] | null),or?: ((InterlinearAlignmentsFilter | null)[] | null),not?: (InterlinearAlignmentsFilter | null)}

export interface InterlinearAlignmentsInput {uid?: (Scalars['ID'] | null),lemmaA?: (LemmasInput | null),lemmaB?: (LemmasInput | null)}

export interface InterlinearAlignmentsSort {id?: (SortDirection | null)}

export interface IntFilter {eq?: (Scalars['Int'] | null),gt?: (Scalars['Int'] | null),lt?: (Scalars['Int'] | null),ge?: (Scalars['Int'] | null),le?: (Scalars['Int'] | null),between?: ((Scalars['Int'] | null)[] | null),in?: ((Scalars['Int'] | null)[] | null)}

export interface KeywordGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    text?: boolean | number
    videos?: VideoKeywordGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface KeywordFilter {text?: (StringFilter | null),and?: ((KeywordFilter | null)[] | null),or?: ((KeywordFilter | null)[] | null),not?: (KeywordFilter | null)}

export interface KeywordInput {uid?: (Scalars['ID'] | null),text?: (Scalars['String'] | null),videos?: ((VideoKeywordInput | null)[] | null)}

export interface KeywordSort {text?: (SortDirection | null)}

export interface LanguageGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    code?: boolean | number
    name?: boolean | number
    translations?: TranslationsGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface LanguageFilter {code?: (StringFilter | null),name?: (StringFilter | null),and?: ((LanguageFilter | null)[] | null),or?: ((LanguageFilter | null)[] | null),not?: (LanguageFilter | null)}

export interface LanguageInput {uid?: (Scalars['ID'] | null),code?: (Scalars['String'] | null),name?: (Scalars['String'] | null),translations?: ((TranslationsInput | null)[] | null)}

export interface LanguageSort {code?: (SortDirection | null),name?: (SortDirection | null)}

export interface LemmasGenqlSelection{
    uid?: boolean | number
    lemmaId?: boolean | number
    strongsId?: boolean | number
    languageCode?: boolean | number
    lemmaText?: boolean | number
    gloss?: boolean | number
    wordnetLinks?: WordnetLinksGenqlSelection
    lexiconEmbeddings?: LexiconEmbeddingsGenqlSelection
    syntaxRelationsSub?: SyntaxRelationsGenqlSelection
    syntaxRelationsVerb?: SyntaxRelationsGenqlSelection
    syntaxRelationsObj?: SyntaxRelationsGenqlSelection
    interlinearAlignmentsA?: InterlinearAlignmentsGenqlSelection
    interlinearAlignmentsB?: InterlinearAlignmentsGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface LemmasFilter {lemmaId?: (IntFilter | null),strongsId?: (StringFilter | null),languageCode?: (StringFilter | null),lemmaText?: (StringFilter | null),gloss?: (StringFilter | null),and?: ((LemmasFilter | null)[] | null),or?: ((LemmasFilter | null)[] | null),not?: (LemmasFilter | null)}

export interface LemmasInput {uid?: (Scalars['ID'] | null),lemmaId?: (Scalars['Int'] | null),strongsId?: (Scalars['String'] | null),languageCode?: (Scalars['String'] | null),lemmaText?: (Scalars['String'] | null),gloss?: (Scalars['String'] | null),wordnetLinks?: ((WordnetLinksInput | null)[] | null),lexiconEmbeddings?: ((LexiconEmbeddingsInput | null)[] | null),syntaxRelationsSub?: ((SyntaxRelationsInput | null)[] | null),syntaxRelationsVerb?: ((SyntaxRelationsInput | null)[] | null),syntaxRelationsObj?: ((SyntaxRelationsInput | null)[] | null),interlinearAlignmentsA?: ((InterlinearAlignmentsInput | null)[] | null),interlinearAlignmentsB?: ((InterlinearAlignmentsInput | null)[] | null)}

export interface LemmasSort {lemmaId?: (SortDirection | null),strongsId?: (SortDirection | null),languageCode?: (SortDirection | null),lemmaText?: (SortDirection | null),gloss?: (SortDirection | null)}

export interface LexiconEmbeddingsGenqlSelection{
    uid?: boolean | number
    embeddingId?: boolean | number
    modelName?: boolean | number
    lemma?: LemmasGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface LexiconEmbeddingsFilter {embeddingId?: (IntFilter | null),modelName?: (StringFilter | null),and?: ((LexiconEmbeddingsFilter | null)[] | null),or?: ((LexiconEmbeddingsFilter | null)[] | null),not?: (LexiconEmbeddingsFilter | null)}

export interface LexiconEmbeddingsInput {uid?: (Scalars['ID'] | null),embeddingId?: (Scalars['Int'] | null),modelName?: (Scalars['String'] | null),lemma?: (LemmasInput | null)}

export interface LexiconEmbeddingsSort {embeddingId?: (SortDirection | null),modelName?: (SortDirection | null)}

export interface MutationGenqlSelection{
    createLanguage?: (LanguageGenqlSelection & { __args: {input: LanguageInput} })
    updateLanguage?: { __args: {uid: Scalars['ID'], input: LanguageInput} }
    deleteLanguage?: { __args: {uid: Scalars['ID']} }
    createCategory?: (CategoryGenqlSelection & { __args: {input: CategoryInput} })
    updateCategory?: { __args: {uid: Scalars['ID'], input: CategoryInput} }
    deleteCategory?: { __args: {uid: Scalars['ID']} }
    createTranslations?: (TranslationsGenqlSelection & { __args: {input: TranslationsInput} })
    updateTranslations?: { __args: {uid: Scalars['ID'], input: TranslationsInput} }
    deleteTranslations?: { __args: {uid: Scalars['ID']} }
    createBook?: (BookGenqlSelection & { __args: {input: BookInput} })
    updateBook?: { __args: {uid: Scalars['ID'], input: BookInput} }
    deleteBook?: { __args: {uid: Scalars['ID']} }
    createBookTranslation?: (BookTranslationGenqlSelection & { __args: {input: BookTranslationInput} })
    updateBookTranslation?: { __args: {uid: Scalars['ID'], input: BookTranslationInput} }
    deleteBookTranslation?: { __args: {uid: Scalars['ID']} }
    createBookCategory?: (BookCategoryGenqlSelection & { __args: {input: BookCategoryInput} })
    updateBookCategory?: { __args: {uid: Scalars['ID'], input: BookCategoryInput} }
    deleteBookCategory?: { __args: {uid: Scalars['ID']} }
    createChapter?: (ChapterGenqlSelection & { __args: {input: ChapterInput} })
    updateChapter?: { __args: {uid: Scalars['ID'], input: ChapterInput} }
    deleteChapter?: { __args: {uid: Scalars['ID']} }
    createVerse?: (VerseGenqlSelection & { __args: {input: VerseInput} })
    updateVerse?: { __args: {uid: Scalars['ID'], input: VerseInput} }
    deleteVerse?: { __args: {uid: Scalars['ID']} }
    createVerseContent?: (VerseContentGenqlSelection & { __args: {input: VerseContentInput} })
    updateVerseContent?: { __args: {uid: Scalars['ID'], input: VerseContentInput} }
    deleteVerseContent?: { __args: {uid: Scalars['ID']} }
    createLemmas?: (LemmasGenqlSelection & { __args: {input: LemmasInput} })
    updateLemmas?: { __args: {uid: Scalars['ID'], input: LemmasInput} }
    deleteLemmas?: { __args: {uid: Scalars['ID']} }
    createTokenMorphology?: (TokenMorphologyGenqlSelection & { __args: {input: TokenMorphologyInput} })
    updateTokenMorphology?: { __args: {uid: Scalars['ID'], input: TokenMorphologyInput} }
    deleteTokenMorphology?: { __args: {uid: Scalars['ID']} }
    createWordnetLinks?: (WordnetLinksGenqlSelection & { __args: {input: WordnetLinksInput} })
    updateWordnetLinks?: { __args: {uid: Scalars['ID'], input: WordnetLinksInput} }
    deleteWordnetLinks?: { __args: {uid: Scalars['ID']} }
    createVerseEmbeddings?: (VerseEmbeddingsGenqlSelection & { __args: {input: VerseEmbeddingsInput} })
    updateVerseEmbeddings?: { __args: {uid: Scalars['ID'], input: VerseEmbeddingsInput} }
    deleteVerseEmbeddings?: { __args: {uid: Scalars['ID']} }
    createLexiconEmbeddings?: (LexiconEmbeddingsGenqlSelection & { __args: {input: LexiconEmbeddingsInput} })
    updateLexiconEmbeddings?: { __args: {uid: Scalars['ID'], input: LexiconEmbeddingsInput} }
    deleteLexiconEmbeddings?: { __args: {uid: Scalars['ID']} }
    createSummaries?: (SummariesGenqlSelection & { __args: {input: SummariesInput} })
    updateSummaries?: { __args: {uid: Scalars['ID'], input: SummariesInput} }
    deleteSummaries?: { __args: {uid: Scalars['ID']} }
    createSyntaxRelations?: (SyntaxRelationsGenqlSelection & { __args: {input: SyntaxRelationsInput} })
    updateSyntaxRelations?: { __args: {uid: Scalars['ID'], input: SyntaxRelationsInput} }
    deleteSyntaxRelations?: { __args: {uid: Scalars['ID']} }
    createInterlinearAlignments?: (InterlinearAlignmentsGenqlSelection & { __args: {input: InterlinearAlignmentsInput} })
    updateInterlinearAlignments?: { __args: {uid: Scalars['ID'], input: InterlinearAlignmentsInput} }
    deleteInterlinearAlignments?: { __args: {uid: Scalars['ID']} }
    createEntities?: (EntitiesGenqlSelection & { __args: {input: EntitiesInput} })
    updateEntities?: { __args: {uid: Scalars['ID'], input: EntitiesInput} }
    deleteEntities?: { __args: {uid: Scalars['ID']} }
    createEntityMentions?: (EntityMentionsGenqlSelection & { __args: {input: EntityMentionsInput} })
    updateEntityMentions?: { __args: {uid: Scalars['ID'], input: EntityMentionsInput} }
    deleteEntityMentions?: { __args: {uid: Scalars['ID']} }
    createEntityEdges?: (EntityEdgesGenqlSelection & { __args: {input: EntityEdgesInput} })
    updateEntityEdges?: { __args: {uid: Scalars['ID'], input: EntityEdgesInput} }
    deleteEntityEdges?: { __args: {uid: Scalars['ID']} }
    createUnifiedBible?: (UnifiedBibleGenqlSelection & { __args: {input: UnifiedBibleInput} })
    updateUnifiedBible?: { __args: {uid: Scalars['ID'], input: UnifiedBibleInput} }
    deleteUnifiedBible?: { __args: {uid: Scalars['ID']} }
    createVideo?: (VideoGenqlSelection & { __args: {input: VideoInput} })
    updateVideo?: { __args: {uid: Scalars['ID'], input: VideoInput} }
    deleteVideo?: { __args: {uid: Scalars['ID']} }
    createVideoView?: (VideoViewGenqlSelection & { __args: {input: VideoViewInput} })
    updateVideoView?: { __args: {uid: Scalars['ID'], input: VideoViewInput} }
    deleteVideoView?: { __args: {uid: Scalars['ID']} }
    createKeyword?: (KeywordGenqlSelection & { __args: {input: KeywordInput} })
    updateKeyword?: { __args: {uid: Scalars['ID'], input: KeywordInput} }
    deleteKeyword?: { __args: {uid: Scalars['ID']} }
    createVideoKeyword?: (VideoKeywordGenqlSelection & { __args: {input: VideoKeywordInput} })
    updateVideoKeyword?: { __args: {uid: Scalars['ID'], input: VideoKeywordInput} }
    deleteVideoKeyword?: { __args: {uid: Scalars['ID']} }
    flushDatabase?: boolean | number
    compactDatabase?: boolean | number
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface MutationEventGenqlSelection{
    type?: boolean | number
    uid?: boolean | number
    mutation?: boolean | number
    payload?: boolean | number
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface QueryGenqlSelection{
    queryLanguage?: (LanguageGenqlSelection & { __args?: {filter?: (LanguageFilter | null), sort?: (LanguageSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getLanguage?: (LanguageGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), code?: (Scalars['String'] | null)} })
    queryCategory?: (CategoryGenqlSelection & { __args?: {filter?: (CategoryFilter | null), sort?: (CategorySort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getCategory?: (CategoryGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), name?: (Scalars['String'] | null)} })
    queryTranslations?: (TranslationsGenqlSelection & { __args?: {filter?: (TranslationsFilter | null), sort?: (TranslationsSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getTranslations?: (TranslationsGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), code?: (Scalars['String'] | null)} })
    queryBook?: (BookGenqlSelection & { __args?: {filter?: (BookFilter | null), sort?: (BookSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getBook?: (BookGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), code?: (Scalars['String'] | null)} })
    queryBookTranslation?: (BookTranslationGenqlSelection & { __args?: {filter?: (BookTranslationFilter | null), sort?: (BookTranslationSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getBookTranslation?: (BookTranslationGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryBookCategory?: (BookCategoryGenqlSelection & { __args?: {filter?: (BookCategoryFilter | null), sort?: (BookCategorySort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getBookCategory?: (BookCategoryGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryChapter?: (ChapterGenqlSelection & { __args?: {filter?: (ChapterFilter | null), sort?: (ChapterSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getChapter?: (ChapterGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryVerse?: (VerseGenqlSelection & { __args?: {filter?: (VerseFilter | null), sort?: (VerseSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getVerse?: (VerseGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryVerseContent?: (VerseContentGenqlSelection & { __args?: {filter?: (VerseContentFilter | null), sort?: (VerseContentSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getVerseContent?: (VerseContentGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryLemmas?: (LemmasGenqlSelection & { __args?: {filter?: (LemmasFilter | null), sort?: (LemmasSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getLemmas?: (LemmasGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), lemmaId?: (Scalars['String'] | null)} })
    queryTokenMorphology?: (TokenMorphologyGenqlSelection & { __args?: {filter?: (TokenMorphologyFilter | null), sort?: (TokenMorphologySort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getTokenMorphology?: (TokenMorphologyGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), morphId?: (Scalars['String'] | null)} })
    queryWordnetLinks?: (WordnetLinksGenqlSelection & { __args?: {filter?: (WordnetLinksFilter | null), sort?: (WordnetLinksSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getWordnetLinks?: (WordnetLinksGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryVerseEmbeddings?: (VerseEmbeddingsGenqlSelection & { __args?: {filter?: (VerseEmbeddingsFilter | null), sort?: (VerseEmbeddingsSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getVerseEmbeddings?: (VerseEmbeddingsGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryLexiconEmbeddings?: (LexiconEmbeddingsGenqlSelection & { __args?: {filter?: (LexiconEmbeddingsFilter | null), sort?: (LexiconEmbeddingsSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getLexiconEmbeddings?: (LexiconEmbeddingsGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), embeddingId?: (Scalars['String'] | null)} })
    querySummaries?: (SummariesGenqlSelection & { __args?: {filter?: (SummariesFilter | null), sort?: (SummariesSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getSummaries?: (SummariesGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), summaryId?: (Scalars['String'] | null)} })
    querySyntaxRelations?: (SyntaxRelationsGenqlSelection & { __args?: {filter?: (SyntaxRelationsFilter | null), sort?: (SyntaxRelationsSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getSyntaxRelations?: (SyntaxRelationsGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), relationId?: (Scalars['String'] | null)} })
    queryInterlinearAlignments?: (InterlinearAlignmentsGenqlSelection & { __args?: {filter?: (InterlinearAlignmentsFilter | null), sort?: (InterlinearAlignmentsSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getInterlinearAlignments?: (InterlinearAlignmentsGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryEntities?: (EntitiesGenqlSelection & { __args?: {filter?: (EntitiesFilter | null), sort?: (EntitiesSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getEntities?: (EntitiesGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), entityId?: (Scalars['String'] | null)} })
    queryEntityMentions?: (EntityMentionsGenqlSelection & { __args?: {filter?: (EntityMentionsFilter | null), sort?: (EntityMentionsSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getEntityMentions?: (EntityMentionsGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), mentionId?: (Scalars['String'] | null)} })
    queryEntityEdges?: (EntityEdgesGenqlSelection & { __args?: {filter?: (EntityEdgesFilter | null), sort?: (EntityEdgesSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getEntityEdges?: (EntityEdgesGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), edgeId?: (Scalars['String'] | null)} })
    queryUnifiedBible?: (UnifiedBibleGenqlSelection & { __args?: {filter?: (UnifiedBibleFilter | null), sort?: (UnifiedBibleSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getUnifiedBible?: (UnifiedBibleGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryVideo?: (VideoGenqlSelection & { __args?: {filter?: (VideoFilter | null), sort?: (VideoSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getVideo?: (VideoGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryVideoView?: (VideoViewGenqlSelection & { __args?: {filter?: (VideoViewFilter | null), sort?: (VideoViewSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getVideoView?: (VideoViewGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    queryKeyword?: (KeywordGenqlSelection & { __args?: {filter?: (KeywordFilter | null), sort?: (KeywordSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getKeyword?: (KeywordGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null), text?: (Scalars['String'] | null)} })
    queryVideoKeyword?: (VideoKeywordGenqlSelection & { __args?: {filter?: (VideoKeywordFilter | null), sort?: (VideoKeywordSort | null), first?: (Scalars['Int'] | null), after?: (Scalars['String'] | null)} })
    getVideoKeyword?: (VideoKeywordGenqlSelection & { __args?: {uid?: (Scalars['ID'] | null)} })
    search?: (SearchResultGenqlSelection & { __args?: {vector?: (Scalars['Float'][] | null), k?: (Scalars['Int'] | null)} })
    hybridSearch?: (SearchResultGenqlSelection & { __args: {vector?: (Scalars['Float'][] | null), text: Scalars['String'], field: Scalars['String'], k?: (Scalars['Int'] | null)} })
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface SearchResultGenqlSelection{
    uid?: boolean | number
    distance?: boolean | number
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface StringFilter {eq?: (Scalars['String'] | null),contains?: (Scalars['String'] | null),allofterms?: (Scalars['String'] | null),anyofterms?: (Scalars['String'] | null),alloftext?: (Scalars['String'] | null),anyoftext?: (Scalars['String'] | null),lt?: (Scalars['String'] | null),le?: (Scalars['String'] | null),gt?: (Scalars['String'] | null),ge?: (Scalars['String'] | null),in?: ((Scalars['String'] | null)[] | null)}

export interface SubscriptionGenqlSelection{
    event?: (MutationEventGenqlSelection & { __args?: {types?: ((Scalars['String'] | null)[] | null)} })
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface SummariesGenqlSelection{
    uid?: boolean | number
    summaryId?: boolean | number
    summaryText?: boolean | number
    level?: boolean | number
    book?: BookGenqlSelection
    chapter?: ChapterGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface SummariesFilter {summaryId?: (IntFilter | null),summaryText?: (StringFilter | null),level?: (IntFilter | null),and?: ((SummariesFilter | null)[] | null),or?: ((SummariesFilter | null)[] | null),not?: (SummariesFilter | null)}

export interface SummariesInput {uid?: (Scalars['ID'] | null),summaryId?: (Scalars['Int'] | null),summaryText?: (Scalars['String'] | null),level?: (Scalars['Int'] | null),book?: (BookInput | null),chapter?: (ChapterInput | null)}

export interface SummariesSort {summaryId?: (SortDirection | null),summaryText?: (SortDirection | null),level?: (SortDirection | null)}

export interface SyntaxRelationsGenqlSelection{
    uid?: boolean | number
    relationId?: boolean | number
    subjectText?: boolean | number
    verbText?: boolean | number
    objectText?: boolean | number
    relationType?: boolean | number
    verse?: VerseGenqlSelection
    subjectLemma?: LemmasGenqlSelection
    verbLemma?: LemmasGenqlSelection
    objectLemma?: LemmasGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface SyntaxRelationsFilter {relationId?: (IntFilter | null),subjectText?: (StringFilter | null),verbText?: (StringFilter | null),objectText?: (StringFilter | null),relationType?: (StringFilter | null),and?: ((SyntaxRelationsFilter | null)[] | null),or?: ((SyntaxRelationsFilter | null)[] | null),not?: (SyntaxRelationsFilter | null)}

export interface SyntaxRelationsInput {uid?: (Scalars['ID'] | null),relationId?: (Scalars['Int'] | null),subjectText?: (Scalars['String'] | null),verbText?: (Scalars['String'] | null),objectText?: (Scalars['String'] | null),relationType?: (Scalars['String'] | null),verse?: (VerseInput | null),subjectLemma?: (LemmasInput | null),verbLemma?: (LemmasInput | null),objectLemma?: (LemmasInput | null)}

export interface SyntaxRelationsSort {relationId?: (SortDirection | null),subjectText?: (SortDirection | null),verbText?: (SortDirection | null),objectText?: (SortDirection | null),relationType?: (SortDirection | null)}

export interface TokenMorphologyGenqlSelection{
    uid?: boolean | number
    morphId?: boolean | number
    partOfSpeech?: boolean | number
    person?: boolean | number
    number?: boolean | number
    gender?: boolean | number
    tense?: boolean | number
    mood?: boolean | number
    voice?: boolean | number
    caseVal?: boolean | number
    verseContent?: VerseContentGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface TokenMorphologyFilter {morphId?: (IntFilter | null),partOfSpeech?: (StringFilter | null),person?: (StringFilter | null),number?: (StringFilter | null),gender?: (StringFilter | null),tense?: (StringFilter | null),mood?: (StringFilter | null),voice?: (StringFilter | null),caseVal?: (StringFilter | null),and?: ((TokenMorphologyFilter | null)[] | null),or?: ((TokenMorphologyFilter | null)[] | null),not?: (TokenMorphologyFilter | null)}

export interface TokenMorphologyInput {uid?: (Scalars['ID'] | null),morphId?: (Scalars['Int'] | null),partOfSpeech?: (Scalars['String'] | null),person?: (Scalars['String'] | null),number?: (Scalars['String'] | null),gender?: (Scalars['String'] | null),tense?: (Scalars['String'] | null),mood?: (Scalars['String'] | null),voice?: (Scalars['String'] | null),caseVal?: (Scalars['String'] | null),verseContent?: (VerseContentInput | null)}

export interface TokenMorphologySort {morphId?: (SortDirection | null),partOfSpeech?: (SortDirection | null),person?: (SortDirection | null),number?: (SortDirection | null),gender?: (SortDirection | null),tense?: (SortDirection | null),mood?: (SortDirection | null),voice?: (SortDirection | null),caseVal?: (SortDirection | null)}

export interface TranslationsGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    code?: boolean | number
    name?: boolean | number
    isInterlinear?: boolean | number
    language?: LanguageGenqlSelection
    bookTranslations?: BookTranslationGenqlSelection
    verseContents?: VerseContentGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface TranslationsFilter {code?: (StringFilter | null),name?: (StringFilter | null),isInterlinear?: (BooleanFilter | null),and?: ((TranslationsFilter | null)[] | null),or?: ((TranslationsFilter | null)[] | null),not?: (TranslationsFilter | null)}

export interface TranslationsInput {uid?: (Scalars['ID'] | null),code?: (Scalars['String'] | null),name?: (Scalars['String'] | null),isInterlinear?: (Scalars['Boolean'] | null),language?: (LanguageInput | null),bookTranslations?: ((BookTranslationInput | null)[] | null),verseContents?: ((VerseContentInput | null)[] | null)}

export interface TranslationsSort {code?: (SortDirection | null),name?: (SortDirection | null),isInterlinear?: (SortDirection | null)}

export interface UnifiedBibleGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    verseNumber?: boolean | number
    chapter?: boolean | number
    text?: boolean | number
    gospel?: boolean | number
    sourceVerse?: boolean | number
    notes?: boolean | number
    chapterHeading?: boolean | number
    sectionHeading?: boolean | number
    metadata?: boolean | number
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface UnifiedBibleFilter {verseNumber?: (IntFilter | null),chapter?: (IntFilter | null),text?: (StringFilter | null),gospel?: (StringFilter | null),sourceVerse?: (StringFilter | null),notes?: (StringFilter | null),chapterHeading?: (StringFilter | null),sectionHeading?: (StringFilter | null),metadata?: (StringFilter | null),and?: ((UnifiedBibleFilter | null)[] | null),or?: ((UnifiedBibleFilter | null)[] | null),not?: (UnifiedBibleFilter | null)}

export interface UnifiedBibleInput {uid?: (Scalars['ID'] | null),verseNumber?: (Scalars['Int'] | null),chapter?: (Scalars['Int'] | null),text?: (Scalars['String'] | null),gospel?: (Scalars['String'] | null),sourceVerse?: (Scalars['String'] | null),notes?: (Scalars['String'] | null),chapterHeading?: (Scalars['String'] | null),sectionHeading?: (Scalars['String'] | null),metadata?: (Scalars['String'] | null)}

export interface UnifiedBibleSort {verseNumber?: (SortDirection | null),chapter?: (SortDirection | null),text?: (SortDirection | null),gospel?: (SortDirection | null),sourceVerse?: (SortDirection | null),notes?: (SortDirection | null),chapterHeading?: (SortDirection | null),sectionHeading?: (SortDirection | null),metadata?: (SortDirection | null)}

export interface VerseGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    number?: boolean | number
    chunkType?: boolean | number
    chapterHeading?: boolean | number
    sectionHeading?: boolean | number
    paragraphStart?: boolean | number
    poetryIndent?: boolean | number
    metadata?: boolean | number
    chapter?: ChapterGenqlSelection
    verseContents?: (VerseContentGenqlSelection & { __args?: {filter?: (VerseContentFilter | null)} })
    verseEmbeddings?: VerseEmbeddingsGenqlSelection
    syntaxRelations?: SyntaxRelationsGenqlSelection
    entityMentions?: EntityMentionsGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface VerseContentGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    type?: boolean | number
    text?: boolean | number
    lemma?: boolean | number
    strong?: boolean | number
    morph?: boolean | number
    gloss?: boolean | number
    verse?: VerseGenqlSelection
    translation?: TranslationsGenqlSelection
    tokenMorphologies?: TokenMorphologyGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface VerseContentFilter {type?: (StringFilter | null),text?: (StringFilter | null),lemma?: (StringFilter | null),strong?: (StringFilter | null),morph?: (StringFilter | null),gloss?: (StringFilter | null),and?: ((VerseContentFilter | null)[] | null),or?: ((VerseContentFilter | null)[] | null),not?: (VerseContentFilter | null)}

export interface VerseContentInput {uid?: (Scalars['ID'] | null),type?: (Scalars['String'] | null),text?: (Scalars['String'] | null),lemma?: (Scalars['String'] | null),strong?: (Scalars['String'] | null),morph?: (Scalars['String'] | null),gloss?: (Scalars['String'] | null),verse?: (VerseInput | null),translation?: (TranslationsInput | null),tokenMorphologies?: ((TokenMorphologyInput | null)[] | null)}

export interface VerseContentSort {type?: (SortDirection | null),text?: (SortDirection | null),lemma?: (SortDirection | null),strong?: (SortDirection | null),morph?: (SortDirection | null),gloss?: (SortDirection | null)}

export interface VerseEmbeddingsGenqlSelection{
    uid?: boolean | number
    embeddingId?: boolean | number
    modelName?: boolean | number
    verse?: VerseGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface VerseEmbeddingsFilter {modelName?: (StringFilter | null),and?: ((VerseEmbeddingsFilter | null)[] | null),or?: ((VerseEmbeddingsFilter | null)[] | null),not?: (VerseEmbeddingsFilter | null)}

export interface VerseEmbeddingsInput {uid?: (Scalars['ID'] | null),modelName?: (Scalars['String'] | null),verse?: (VerseInput | null)}

export interface VerseEmbeddingsSort {modelName?: (SortDirection | null)}

export interface VerseFilter {number?: (IntFilter | null),chunkType?: (StringFilter | null),chapterHeading?: (StringFilter | null),sectionHeading?: (StringFilter | null),paragraphStart?: (BooleanFilter | null),poetryIndent?: (IntFilter | null),metadata?: (StringFilter | null),and?: ((VerseFilter | null)[] | null),or?: ((VerseFilter | null)[] | null),not?: (VerseFilter | null)}

export interface VerseInput {uid?: (Scalars['ID'] | null),number?: (Scalars['Int'] | null),chunkType?: (Scalars['String'] | null),chapterHeading?: (Scalars['String'] | null),sectionHeading?: (Scalars['String'] | null),paragraphStart?: (Scalars['Boolean'] | null),poetryIndent?: (Scalars['Int'] | null),metadata?: (Scalars['String'] | null),chapter?: (ChapterInput | null),verseContents?: ((VerseContentInput | null)[] | null),verseEmbeddings?: ((VerseEmbeddingsInput | null)[] | null),syntaxRelations?: ((SyntaxRelationsInput | null)[] | null),entityMentions?: ((EntityMentionsInput | null)[] | null)}

export interface VerseSort {number?: (SortDirection | null),chunkType?: (SortDirection | null),chapterHeading?: (SortDirection | null),sectionHeading?: (SortDirection | null),paragraphStart?: (SortDirection | null),poetryIndent?: (SortDirection | null),metadata?: (SortDirection | null)}

export interface VideoGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    name?: boolean | number
    status?: boolean | number
    createdAt?: boolean | number
    updatedAt?: boolean | number
    script?: boolean | number
    generatedFilename?: boolean | number
    generatedTitle?: boolean | number
    generatedDescription?: boolean | number
    generatedTags?: boolean | number
    thumbnailPromptA?: boolean | number
    thumbnailPromptB?: boolean | number
    checklist?: boolean | number
    views?: VideoViewGenqlSelection
    keywords?: VideoKeywordGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface VideoFilter {name?: (StringFilter | null),status?: (StringFilter | null),createdAt?: (DateTimeFilter | null),updatedAt?: (DateTimeFilter | null),script?: (StringFilter | null),generatedFilename?: (StringFilter | null),generatedTitle?: (StringFilter | null),generatedDescription?: (StringFilter | null),generatedTags?: (StringFilter | null),thumbnailPromptA?: (StringFilter | null),thumbnailPromptB?: (StringFilter | null),checklist?: (StringFilter | null),and?: ((VideoFilter | null)[] | null),or?: ((VideoFilter | null)[] | null),not?: (VideoFilter | null)}

export interface VideoInput {uid?: (Scalars['ID'] | null),name?: (Scalars['String'] | null),status?: (VideoStatus | null),createdAt?: (Scalars['DateTime'] | null),updatedAt?: (Scalars['DateTime'] | null),script?: (Scalars['String'] | null),generatedFilename?: (Scalars['String'] | null),generatedTitle?: (Scalars['String'] | null),generatedDescription?: (Scalars['String'] | null),generatedTags?: (Scalars['String'] | null),thumbnailPromptA?: (Scalars['String'] | null),thumbnailPromptB?: (Scalars['String'] | null),checklist?: (Scalars['String'] | null),views?: ((VideoViewInput | null)[] | null),keywords?: ((VideoKeywordInput | null)[] | null)}

export interface VideoKeywordGenqlSelection{
    uid?: boolean | number
    video?: VideoGenqlSelection
    keyword?: KeywordGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface VideoKeywordFilter {and?: ((VideoKeywordFilter | null)[] | null),or?: ((VideoKeywordFilter | null)[] | null),not?: (VideoKeywordFilter | null)}

export interface VideoKeywordInput {uid?: (Scalars['ID'] | null),video?: (VideoInput | null),keyword?: (KeywordInput | null)}

export interface VideoKeywordSort {id?: (SortDirection | null)}

export interface VideoSort {name?: (SortDirection | null),status?: (SortDirection | null),createdAt?: (SortDirection | null),updatedAt?: (SortDirection | null),script?: (SortDirection | null),generatedFilename?: (SortDirection | null),generatedTitle?: (SortDirection | null),generatedDescription?: (SortDirection | null),generatedTags?: (SortDirection | null),thumbnailPromptA?: (SortDirection | null),thumbnailPromptB?: (SortDirection | null),checklist?: (SortDirection | null)}

export interface VideoViewGenqlSelection{
    uid?: boolean | number
    id?: boolean | number
    date?: boolean | number
    count?: boolean | number
    video?: VideoGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface VideoViewFilter {date?: (DateTimeFilter | null),count?: (IntFilter | null),and?: ((VideoViewFilter | null)[] | null),or?: ((VideoViewFilter | null)[] | null),not?: (VideoViewFilter | null)}

export interface VideoViewInput {uid?: (Scalars['ID'] | null),date?: (Scalars['DateTime'] | null),count?: (Scalars['Int'] | null),video?: (VideoInput | null)}

export interface VideoViewSort {date?: (SortDirection | null),count?: (SortDirection | null)}

export interface WordnetLinksGenqlSelection{
    uid?: boolean | number
    synsetId?: boolean | number
    similarityScore?: boolean | number
    lemma?: LemmasGenqlSelection
    __typename?: boolean | number
    __scalar?: boolean | number
}

export interface WordnetLinksFilter {synsetId?: (StringFilter | null),similarityScore?: (FloatFilter | null),and?: ((WordnetLinksFilter | null)[] | null),or?: ((WordnetLinksFilter | null)[] | null),not?: (WordnetLinksFilter | null)}

export interface WordnetLinksInput {uid?: (Scalars['ID'] | null),synsetId?: (Scalars['String'] | null),similarityScore?: (Scalars['Float'] | null),lemma?: (LemmasInput | null)}

export interface WordnetLinksSort {synsetId?: (SortDirection | null),similarityScore?: (SortDirection | null)}


    const Book_possibleTypes: string[] = ['Book']
    export const isBook = (obj?: { __typename?: any } | null): obj is Book => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isBook"')
      return Book_possibleTypes.includes(obj.__typename)
    }
    


    const BookCategory_possibleTypes: string[] = ['BookCategory']
    export const isBookCategory = (obj?: { __typename?: any } | null): obj is BookCategory => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isBookCategory"')
      return BookCategory_possibleTypes.includes(obj.__typename)
    }
    


    const BookTranslation_possibleTypes: string[] = ['BookTranslation']
    export const isBookTranslation = (obj?: { __typename?: any } | null): obj is BookTranslation => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isBookTranslation"')
      return BookTranslation_possibleTypes.includes(obj.__typename)
    }
    


    const Category_possibleTypes: string[] = ['Category']
    export const isCategory = (obj?: { __typename?: any } | null): obj is Category => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isCategory"')
      return Category_possibleTypes.includes(obj.__typename)
    }
    


    const Chapter_possibleTypes: string[] = ['Chapter']
    export const isChapter = (obj?: { __typename?: any } | null): obj is Chapter => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isChapter"')
      return Chapter_possibleTypes.includes(obj.__typename)
    }
    


    const Entities_possibleTypes: string[] = ['Entities']
    export const isEntities = (obj?: { __typename?: any } | null): obj is Entities => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isEntities"')
      return Entities_possibleTypes.includes(obj.__typename)
    }
    


    const EntityEdges_possibleTypes: string[] = ['EntityEdges']
    export const isEntityEdges = (obj?: { __typename?: any } | null): obj is EntityEdges => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isEntityEdges"')
      return EntityEdges_possibleTypes.includes(obj.__typename)
    }
    


    const EntityMentions_possibleTypes: string[] = ['EntityMentions']
    export const isEntityMentions = (obj?: { __typename?: any } | null): obj is EntityMentions => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isEntityMentions"')
      return EntityMentions_possibleTypes.includes(obj.__typename)
    }
    


    const InterlinearAlignments_possibleTypes: string[] = ['InterlinearAlignments']
    export const isInterlinearAlignments = (obj?: { __typename?: any } | null): obj is InterlinearAlignments => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isInterlinearAlignments"')
      return InterlinearAlignments_possibleTypes.includes(obj.__typename)
    }
    


    const Keyword_possibleTypes: string[] = ['Keyword']
    export const isKeyword = (obj?: { __typename?: any } | null): obj is Keyword => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isKeyword"')
      return Keyword_possibleTypes.includes(obj.__typename)
    }
    


    const Language_possibleTypes: string[] = ['Language']
    export const isLanguage = (obj?: { __typename?: any } | null): obj is Language => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isLanguage"')
      return Language_possibleTypes.includes(obj.__typename)
    }
    


    const Lemmas_possibleTypes: string[] = ['Lemmas']
    export const isLemmas = (obj?: { __typename?: any } | null): obj is Lemmas => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isLemmas"')
      return Lemmas_possibleTypes.includes(obj.__typename)
    }
    


    const LexiconEmbeddings_possibleTypes: string[] = ['LexiconEmbeddings']
    export const isLexiconEmbeddings = (obj?: { __typename?: any } | null): obj is LexiconEmbeddings => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isLexiconEmbeddings"')
      return LexiconEmbeddings_possibleTypes.includes(obj.__typename)
    }
    


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
    


    const SearchResult_possibleTypes: string[] = ['SearchResult']
    export const isSearchResult = (obj?: { __typename?: any } | null): obj is SearchResult => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isSearchResult"')
      return SearchResult_possibleTypes.includes(obj.__typename)
    }
    


    const Subscription_possibleTypes: string[] = ['Subscription']
    export const isSubscription = (obj?: { __typename?: any } | null): obj is Subscription => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isSubscription"')
      return Subscription_possibleTypes.includes(obj.__typename)
    }
    


    const Summaries_possibleTypes: string[] = ['Summaries']
    export const isSummaries = (obj?: { __typename?: any } | null): obj is Summaries => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isSummaries"')
      return Summaries_possibleTypes.includes(obj.__typename)
    }
    


    const SyntaxRelations_possibleTypes: string[] = ['SyntaxRelations']
    export const isSyntaxRelations = (obj?: { __typename?: any } | null): obj is SyntaxRelations => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isSyntaxRelations"')
      return SyntaxRelations_possibleTypes.includes(obj.__typename)
    }
    


    const TokenMorphology_possibleTypes: string[] = ['TokenMorphology']
    export const isTokenMorphology = (obj?: { __typename?: any } | null): obj is TokenMorphology => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isTokenMorphology"')
      return TokenMorphology_possibleTypes.includes(obj.__typename)
    }
    


    const Translations_possibleTypes: string[] = ['Translations']
    export const isTranslations = (obj?: { __typename?: any } | null): obj is Translations => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isTranslations"')
      return Translations_possibleTypes.includes(obj.__typename)
    }
    


    const UnifiedBible_possibleTypes: string[] = ['UnifiedBible']
    export const isUnifiedBible = (obj?: { __typename?: any } | null): obj is UnifiedBible => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isUnifiedBible"')
      return UnifiedBible_possibleTypes.includes(obj.__typename)
    }
    


    const Verse_possibleTypes: string[] = ['Verse']
    export const isVerse = (obj?: { __typename?: any } | null): obj is Verse => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isVerse"')
      return Verse_possibleTypes.includes(obj.__typename)
    }
    


    const VerseContent_possibleTypes: string[] = ['VerseContent']
    export const isVerseContent = (obj?: { __typename?: any } | null): obj is VerseContent => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isVerseContent"')
      return VerseContent_possibleTypes.includes(obj.__typename)
    }
    


    const VerseEmbeddings_possibleTypes: string[] = ['VerseEmbeddings']
    export const isVerseEmbeddings = (obj?: { __typename?: any } | null): obj is VerseEmbeddings => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isVerseEmbeddings"')
      return VerseEmbeddings_possibleTypes.includes(obj.__typename)
    }
    


    const Video_possibleTypes: string[] = ['Video']
    export const isVideo = (obj?: { __typename?: any } | null): obj is Video => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isVideo"')
      return Video_possibleTypes.includes(obj.__typename)
    }
    


    const VideoKeyword_possibleTypes: string[] = ['VideoKeyword']
    export const isVideoKeyword = (obj?: { __typename?: any } | null): obj is VideoKeyword => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isVideoKeyword"')
      return VideoKeyword_possibleTypes.includes(obj.__typename)
    }
    


    const VideoView_possibleTypes: string[] = ['VideoView']
    export const isVideoView = (obj?: { __typename?: any } | null): obj is VideoView => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isVideoView"')
      return VideoView_possibleTypes.includes(obj.__typename)
    }
    


    const WordnetLinks_possibleTypes: string[] = ['WordnetLinks']
    export const isWordnetLinks = (obj?: { __typename?: any } | null): obj is WordnetLinks => {
      if (!obj?.__typename) throw new Error('__typename is missing in "isWordnetLinks"')
      return WordnetLinks_possibleTypes.includes(obj.__typename)
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

export const enumVideoStatus = {
   NEW: 'NEW' as const,
   IN_PROGRESS: 'IN_PROGRESS' as const,
   COMPLETED: 'COMPLETED' as const,
   SCHEDULED: 'SCHEDULED' as const,
   PUBLISHED: 'PUBLISHED' as const,
   DELETED: 'DELETED' as const
}
