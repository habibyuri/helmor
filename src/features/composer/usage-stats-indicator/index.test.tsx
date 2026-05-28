import { cleanup, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createHelmorQueryClient, helmorQueryKeys } from "@/lib/query-client";
import {
	type AppSettings,
	DEFAULT_SETTINGS,
	SettingsContext,
} from "@/lib/settings";
import { renderWithProviders } from "@/test/render-with-providers";
import { UsageStatsIndicator } from "./index";

const apiMocks = vi.hoisted(() => ({
	getCodexRateLimits: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
	const actual = await importOriginal<typeof import("@/lib/api")>();
	return {
		...actual,
		getCodexRateLimits: apiMocks.getCodexRateLimits,
	};
});

const CODEX_USAGE_RAW = JSON.stringify({
	plan_type: "pro",
	rate_limit: {
		primary_window: {
			used_percent: 25,
			limit_window_seconds: 18_000,
			reset_at: 1_777_003_600,
		},
	},
});

function renderCodexIndicator(
	settings: Partial<AppSettings>,
	options: { seedCachedUsage?: boolean } = { seedCachedUsage: true },
) {
	const queryClient = createHelmorQueryClient();
	queryClient.setDefaultOptions({
		queries: {
			...queryClient.getDefaultOptions().queries,
			retry: false,
		},
	});
	if (options.seedCachedUsage ?? true) {
		queryClient.setQueryData(
			helmorQueryKeys.codexRateLimits(settings.codexExecutablePath ?? ""),
			CODEX_USAGE_RAW,
		);
	}

	return renderWithProviders(
		<SettingsContext.Provider
			value={{
				settings: { ...DEFAULT_SETTINGS, ...settings },
				isLoaded: true,
				updateSettings: vi.fn(),
			}}
		>
			<UsageStatsIndicator agentType="codex" />
		</SettingsContext.Provider>,
		{ queryClient },
	);
}

describe("UsageStatsIndicator", () => {
	beforeEach(() => {
		apiMocks.getCodexRateLimits.mockResolvedValue(CODEX_USAGE_RAW);
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it("renders Codex usage affordance while usage details are loading", () => {
		apiMocks.getCodexRateLimits.mockReturnValue(new Promise(() => {}));

		renderCodexIndicator(
			{ codexExecutablePath: "" },
			{ seedCachedUsage: false },
		);

		expect(
			screen.getByRole("button", { name: "Usage Stats" }),
		).toBeInTheDocument();
	});

	it("does not wait on login status to learn usage is unavailable", async () => {
		apiMocks.getCodexRateLimits.mockResolvedValue(null);

		renderCodexIndicator(
			{ codexExecutablePath: "/usr/local/bin/codexaz" },
			{ seedCachedUsage: false },
		);

		await waitFor(() => {
			expect(
				screen.queryByRole("button", { name: "Usage Stats" }),
			).not.toBeInTheDocument();
		});
	});

	it("still renders Codex usage for the normal codex executable name", async () => {
		renderCodexIndicator({ codexExecutablePath: "/usr/local/bin/codexaz" });

		await waitFor(() => {
			expect(
				screen.getByRole("button", { name: "Usage Stats" }),
			).toBeInTheDocument();
		});
	});
});
