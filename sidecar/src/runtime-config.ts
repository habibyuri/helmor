import { sanitizeEnvMap } from "./codex-binary-lifecycle.js";

export type RuntimeConfigPatch = {
	cursorApiKey?: string | null;
	codexBinaryPath?: string | null;
	codexConfigPath?: string | null;
	codexEnv?: Record<string, string>;
};

export function parseRuntimeConfigPatch(
	params: Record<string, unknown>,
): RuntimeConfigPatch {
	const patch: RuntimeConfigPatch = {};
	if ("cursorApiKey" in params) {
		patch.cursorApiKey =
			typeof params.cursorApiKey === "string" ? params.cursorApiKey : null;
	}
	if ("codexBinaryPath" in params) {
		patch.codexBinaryPath =
			typeof params.codexBinaryPath === "string"
				? params.codexBinaryPath
				: null;
	}
	if ("codexConfigPath" in params) {
		patch.codexConfigPath =
			typeof params.codexConfigPath === "string"
				? params.codexConfigPath
				: null;
	}
	if ("codexEnv" in params) {
		patch.codexEnv = sanitizeEnvMap(parseStringMap(params.codexEnv));
	}
	return patch;
}

function parseStringMap(value: unknown): Record<string, string> {
	if (!value || typeof value !== "object" || Array.isArray(value)) return {};
	const out: Record<string, string> = {};
	for (const [key, raw] of Object.entries(value)) {
		if (typeof raw === "string") out[key] = raw;
	}
	return out;
}
