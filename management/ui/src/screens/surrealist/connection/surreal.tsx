import { surreal } from "../../../adapter";

export interface SurrealOptions {
	strict?: boolean;
}

/**
 * Create a new configured Surreal instance
 */
export async function createSurreal(options?: SurrealOptions): Promise<any> {
	return surreal;
}
