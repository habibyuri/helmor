import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders } from "@/test/render-with-providers";
import { CodexBinarySettingsRow } from "./codex-binary";

const apiMocks = vi.hoisted(() => ({
	resolveSystemAgentBinary: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
	open: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
	const actual = await importOriginal<typeof import("@/lib/api")>();
	return {
		...actual,
		resolveSystemAgentBinary: apiMocks.resolveSystemAgentBinary,
	};
});

vi.mock("sonner", () => ({
	toast: Object.assign(vi.fn(), {
		error: vi.fn(),
		success: vi.fn(),
	}),
}));

describe("CodexBinarySettingsRow", () => {
	beforeEach(() => {
		apiMocks.resolveSystemAgentBinary.mockReset();
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it("writes the resolved system Codex path into the setting", async () => {
		apiMocks.resolveSystemAgentBinary.mockResolvedValue(
			"/opt/homebrew/bin/codex",
		);
		const onChange = vi.fn();
		const user = userEvent.setup();

		renderWithProviders(
			<CodexBinarySettingsRow value="" onChange={onChange} />,
		);

		await user.click(
			screen.getByRole("button", { name: "Use system Codex (from PATH)" }),
		);

		await waitFor(() => {
			expect(onChange).toHaveBeenCalledWith("/opt/homebrew/bin/codex");
		});
		expect(apiMocks.resolveSystemAgentBinary).toHaveBeenCalledWith("codex");
		expect(
			screen.getByDisplayValue("/opt/homebrew/bin/codex"),
		).toBeInTheDocument();
		expect(onChange).not.toHaveBeenCalledWith("codex");
	});
});
