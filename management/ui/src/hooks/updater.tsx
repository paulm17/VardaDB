import { useState } from "react";
import { useInterfaceStore } from "~/stores/interface";
import { useStable } from "./stable";

type Phase = "idle" | "downloading" | "error";

function extractMajor(version: string) {
	return Number.parseInt(version.split(".")[0] ?? 0);
}

/**
 * Provides the updater dialog logic for the desktop app.
 */
export function useDesktopUpdater() {
	const update = useInterfaceStore((s) => s.availableUpdate);
	const [phase] = useState<Phase>("idle");

	const startUpdate = useStable(() => {
		console.warn("Update not supported on web");
	});

	const progress = "0";
	const version = update?.version || "";

	return {
		phase,
		progress,
		version,
		startUpdate,
	} as const;
}
