import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useMemo, useState } from "react";
import {
	claudeRateLimitsQueryOptions,
	codexRateLimitsQueryOptions,
	codexRuntimeScope,
	helmorQueryKeys,
} from "@/lib/query-client";
import { useSettings } from "@/lib/settings";
import {
	parseClaudeRateLimits,
	parseCodexRateLimits,
} from "../context-usage-ring/parse";

type UsageStatsAgent = "claude" | "codex" | "cursor" | null;
type UsageStats = ReturnType<typeof parseClaudeRateLimits>;

export function useUsageStats({
	agentType,
	disabled,
}: {
	agentType: UsageStatsAgent;
	disabled?: boolean;
}) {
	const { settings } = useSettings();
	const [open, setOpen] = useState(false);
	const queryClient = useQueryClient();
	const codexScope = codexRuntimeScope(settings);
	const show =
		settings.showUsageStats &&
		(agentType === "claude" || agentType === "codex");

	const codexQuery = useQuery(
		codexRateLimitsQueryOptions(
			show && !disabled && agentType === "codex",
			codexScope.executablePath,
		),
	);
	const codexRaw = codexQuery.data ?? null;
	const { data: claudeRaw = null } = useQuery(
		claudeRateLimitsQueryOptions(show && !disabled && agentType === "claude"),
	);

	const stats = useMemo<UsageStats>(() => {
		if (agentType === "claude") return parseClaudeRateLimits(claudeRaw);
		if (agentType === "codex") return parseCodexRateLimits(codexRaw);
		return null;
	}, [agentType, claudeRaw, codexRaw]);

	const handleOpenChange = useCallback(
		(next: boolean) => {
			setOpen(next);
			if (!next || disabled) return;
			const key =
				agentType === "claude"
					? helmorQueryKeys.claudeRateLimits
					: agentType === "codex"
						? helmorQueryKeys.codexRateLimits(codexScope.executablePath)
						: null;
			if (key) {
				void queryClient.refetchQueries({ queryKey: key });
			}
		},
		[agentType, codexScope.executablePath, disabled, queryClient],
	);

	return {
		available:
			show &&
			(agentType !== "codex" || !codexQuery.isFetched || codexRaw !== null),
		isLoading: agentType === "codex" && codexQuery.isPending,
		open,
		stats,
		onOpenChange: handleOpenChange,
	};
}
