import { getHotkeyHandler } from "@mantine/hooks";
import { compareVersions } from "compare-versions";
import { VIEW_PAGES } from "~/constants";
import { CloudStore } from "~/stores/cloud";
import { ConfigStore, useConfigStore } from "~/stores/config";
import { useDatabaseStore } from "~/stores/database";
import { useInterfaceStore } from "~/stores/interface";
import type { Platform, QueryTab, SurrealistConfig, ViewPage } from "~/types";
import { startCloudSync, syncCloudStore } from "~/util/cloud";
import { getSetting, overwriteConfig, watchStore } from "~/util/config";
import { getConnection } from "~/util/connection";
import { featureFlags } from "~/util/feature-flags";
import { NavigateViewEvent } from "~/util/global-events";
import { showErrorNotification, showInfo } from "~/util/helpers";
import { dispatchIntent, handleIntentRequest } from "~/util/intents";
import { adapter } from ".";
import type { OpenedBinaryFile, OpenedTextFile, SurrealistAdapter } from "./base";

const WAIT_DURATION = 1000;
interface Resource {
	File?: FileResource;
	Link?: LinkResource;
}

interface FileResource {
	success: boolean;
	name: string;
	path: string;
}

interface LinkResource {
	host: string;
	params: string;
}

/**
 * Surrealist adapter for running as Wails desktop app
 */
export class DesktopAdapter implements SurrealistAdapter {
	public readonly id: string = "desktop";

	public isServeSupported = false;
	public isUpdateCheckSupported = false;
	public isTelemetryEnabled = true;
	public isSampleSandboxEnabled = true;
	public titlebarOffset = 0;
	public platform: Platform = "windows";

	public constructor() {
		// No-op for web
	}

	public async initialize() {
		// No-op for web
	}

	public dumpDebug = () => ({
		Platform: "Desktop (Stub)",
		OS: "Web",
		Architecture: "Web",
		WebView: navigator.userAgent,
	});

	public async setWindowTitle(title: string) {
		document.title = title || "Surrealist";
	}

	public async loadConfig() {
		return {};
	}

	public async processConfig(config: SurrealistConfig) {
		return config;
	}

	public saveConfig(config: SurrealistConfig) {
		// No-op
	}

	public async startDatabase() {
		throw new Error("Not supported");
	}

	public stopDatabase() {
		// No-op
	}

	public async openUrl(url: string) {
		window.open(url, "_blank");
	}

	public async saveFile(
		title: string,
		defaultPath: string,
		filters: any,
		content: () => Result<string | Blob | null>,
	): Promise<boolean> {
		return false;
	}

	public async openTextFile<M extends boolean>(
		title: string,
		filters: any,
		multiple: M,
	): Promise<OpenedTextFile[]> {
		return [];
	}

	public async openBinaryFile<M extends boolean>(
		title: string,
		filters: any,
		multiple: M,
	): Promise<OpenedBinaryFile[]> {
		return [];
	}

	public toggleDevTools() {
		// No-op
	}

	public log(label: string, message: string) {
		console.info(`${label}: ${message}`);
	}

	public warn(label: string, message: string) {
		console.warn(`${label}: ${message}`);
	}

	public trace(label: string, message: string) {
		console.debug(`${label}: ${message}`);
	}

	public fetch(url: string, options?: RequestInit | undefined): Promise<Response> {
		return fetch(url, options);
	}

	public async checkForUpdates(force?: boolean) {
		// No-op
	}

	public readQueryFile(query: QueryTab) {
		return Promise.resolve("");
	}

	public writeQueryFile(query: QueryTab, content: string) {
		return Promise.resolve();
	}

	public openQueryFile() {
		return Promise.resolve("");
	}

	public openInExplorer(query: QueryTab) {
		// No-op
	}

	public pruneQueryFiles() {
		// No-op
	}

	public async trackEvent(url: string): Promise<void> {
		const stripCookie = (cookie: string) =>
			cookie
				.split(";")
				.map((c) => c.trim())
				.filter(
					(c) =>
						!c.startsWith("HttpOnly") &&
						!c.startsWith("Secure") &&
						!c.startsWith("Domain"),
				)
				.join("; ");

		const { gtm_debug } = featureFlags.store;
		const previewHeader = getSetting("gtm", "preview_header");

		try {
			// Mock track event
		} catch (err) {
			console.error("Failed to track event: ", err);
		}
	}
}
