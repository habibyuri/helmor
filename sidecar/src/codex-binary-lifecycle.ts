import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

export type RecyclableCodexContext = {
	binaryPath?: string;
	activeTurnId: string | null;
	turnResolve: (() => void) | null;
	turnReject: ((err: Error) => void) | null;
	server: {
		kill(): void;
	};
};

export type RecycleDecision = "keep" | "defer" | "recycled";

/**
 * Resolve the default path to the Codex native binary, used as the spawn
 * target for `codex app-server` child processes when Helmor has no explicit
 * user override.
 *
 * Resolution order:
 *   1. `HELMOR_CODEX_BIN_PATH` — set by the Tauri host in release builds,
 *      pointing at `Helmor.app/Contents/Resources/vendor/codex/codex`.
 *   2. `createRequire` lookup of the platform sub-package's binary inside
 *      `node_modules`. Used in dev (`bun run src/index.ts`) and `bun test`.
 *   3. Fall back to `"codex"` so the OS resolves it from PATH — last-resort
 *      for unusual setups; surfaces as ENOENT if not installed.
 */
export function resolveDefaultCodexBinPath(): string {
	const override = process.env.HELMOR_CODEX_BIN_PATH?.trim();
	if (override) {
		return override;
	}
	const triple = codexTargetTriple();
	if (triple) {
		const platformPkg = `@openai/codex-${platformShort()}`;
		try {
			const require = createRequire(import.meta.url);
			const pkgJson = require.resolve(`${platformPkg}/package.json`);
			const candidate = join(
				dirname(pkgJson),
				"vendor",
				triple,
				"codex",
				process.platform === "win32" ? "codex.exe" : "codex",
			);
			if (existsSync(candidate)) {
				return candidate;
			}
		} catch {
			// Platform sub-package missing (e.g. --omit=optional) — fall through.
		}
	}
	return "codex";
}

export function resolveCodexBinPath(override?: string | null): string {
	const trimmed = override?.trim();
	return trimmed || resolveDefaultCodexBinPath();
}

export function isContextActive(ctx: RecyclableCodexContext): boolean {
	return Boolean(ctx.turnResolve || ctx.turnReject || ctx.activeTurnId);
}

export function sanitizeEnvMap(
	env: Record<string, string> | null,
): Record<string, string> {
	if (!env) return {};
	const sanitized: Record<string, string> = {};
	for (const [key, value] of Object.entries(env)) {
		if (!key || typeof value !== "string" || value.trim() === "") continue;
		sanitized[key] = value;
	}
	return sanitized;
}

export function recycleContextIfStale(
	ctx: RecyclableCodexContext,
	nextBinary: string,
	previousDefault: string,
	onRecycle: () => void,
): RecycleDecision {
	const ctxBinary = ctx.binaryPath ?? previousDefault;
	if (ctxBinary === nextBinary) return "keep";
	if (isContextActive(ctx)) return "defer";
	ctx.server.kill();
	onRecycle();
	return "recycled";
}

function platformShort(): string {
	const arch = process.arch === "x64" ? "x64" : "arm64";
	if (process.platform === "darwin") return `darwin-${arch}`;
	if (process.platform === "linux") return `linux-${arch}`;
	if (process.platform === "win32") return `win32-${arch}`;
	return "";
}

function codexTargetTriple(): string | null {
	const arch = process.arch;
	if (process.platform === "darwin") {
		return arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
	}
	if (process.platform === "linux") {
		return arch === "arm64"
			? "aarch64-unknown-linux-musl"
			: "x86_64-unknown-linux-musl";
	}
	if (process.platform === "win32") {
		return arch === "x64"
			? "x86_64-pc-windows-msvc"
			: "aarch64-pc-windows-msvc";
	}
	return null;
}
