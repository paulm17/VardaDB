// @ts-nocheck
export class GenqlError extends Error {
    constructor(errors, data) {
        let message = Array.isArray(errors)
            ? errors.map((x) => x?.message || '').join('\n')
            : '';
        if (!message) {
            message = 'GraphQL error';
        }
        super(message);
        Object.defineProperty(this, "errors", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: []
        });
        /**
         * Partial data returned by the server
         */
        Object.defineProperty(this, "data", {
            enumerable: true,
            configurable: true,
            writable: true,
            value: void 0
        });
        this.errors = errors;
        this.data = data;
    }
}
