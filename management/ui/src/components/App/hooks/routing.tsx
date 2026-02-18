import { useLayoutEffect } from "react";
import { useAbsoluteLocation, useSearchParams } from "~/hooks/routing";
import { useConfigStore } from "~/stores/config";
import { tagEvent } from "~/util/analytics";
import { handleIntentRequest } from "~/util/intents";
import { AWS_MARKETPLACE_KEY, INVITATION_KEY, REFERRER_KEY } from "~/util/storage";

export function useAppRouter() {
	const { setActiveResource } = useConfigStore.getState();

	const [path, setPath] = useAbsoluteLocation();
	const { intent, referrer, aws_token, invitation } = useSearchParams();
	const resource = useConfigStore((s) => s.activeResource);

	// Restore active resource
	useLayoutEffect(() => {
		if (path === "/") {
			if (!resource || resource === "/") {
				setPath("/management/overview");
			} else {
				const target = resource.startsWith("/") ? resource : `/${resource}`;
				setPath(target.startsWith("/management") ? target : `/management${target}`);
			}
		} else {
			// Save the relative path? Or absolute?
			// But `path` from `useAbsoluteLocation` is it relative?
			// The hook name implies absolute...

			// Wait, `useAbsoluteLocation` in `src/hooks/routing.ts` calls `router.hook(router)`.
			// If `Router` has base, `useLocation` returns path *relative* to base?
			// Wouter docs: "useLocation returns the location relative to the router's base".

			// So `path` is relative.
			// If I save `/overview`, and then next time `setPath("~/overview")`, it works.

			setActiveResource(path);
		}
	}, [path, resource, setActiveResource]);

	// Handle intent requests
	useLayoutEffect(() => {
		if (intent) {
			handleIntentRequest(intent);
		}
	}, [intent]);

	// Cloud referral codes
	useLayoutEffect(() => {
		if (referrer) {
			sessionStorage.setItem(REFERRER_KEY, referrer);
		}
	}, [referrer]);

	// Cloud invitation codes
	useLayoutEffect(() => {
		if (invitation) {
			sessionStorage.setItem(INVITATION_KEY, invitation);
		}
	}, [invitation]);

	// Cloud AWS marketplace token
	useLayoutEffect(() => {
		if (aws_token) {
			sessionStorage.setItem(AWS_MARKETPLACE_KEY, aws_token);
		}
	}, [aws_token]);

	// biome-ignore lint/correctness/useExhaustiveDependencies: Page views
	useLayoutEffect(() => {
		tagEvent("page_view");
	}, [path]);
}
