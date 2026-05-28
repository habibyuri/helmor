import { ClaudeIcon, OpenAIIcon } from "@/components/icons";
import {
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "@/components/ui/hover-card";
import { cn } from "@/lib/utils";
import {
	type RateLimitWindowDisplay,
	ringTier,
} from "../context-usage-ring/parse";
import { LimitRow } from "../context-usage-ring/popover-parts";
import { useUsageStats } from "./use-usage-stats";

type Props = {
	agentType: "claude" | "codex" | "cursor" | null;
	disabled?: boolean;
	className?: string;
};

const HOVER_OPEN_DELAY_MS = 180;
const HOVER_CLOSE_DELAY_MS = 80;

export function UsageStatsIndicator({ agentType, disabled, className }: Props) {
	const { available, isLoading, open, stats, onOpenChange } = useUsageStats({
		agentType,
		disabled,
	});

	if (!available) return null;
	if (
		stats &&
		!stats.primary &&
		!stats.secondary &&
		stats.extraWindows.length === 0 &&
		stats.notes.length === 0
	) {
		return null;
	}

	return (
		<HoverCard
			open={open}
			onOpenChange={onOpenChange}
			openDelay={HOVER_OPEN_DELAY_MS}
			closeDelay={HOVER_CLOSE_DELAY_MS}
		>
			<HoverCardTrigger asChild>
				<button
					type="button"
					disabled={disabled}
					aria-label="Usage Stats"
					className={cn(
						"flex size-7 cursor-interactive items-center justify-center rounded-md disabled:cursor-not-allowed disabled:opacity-50",
						className,
					)}
				>
					<UsageStatsGlyph
						primary={stats?.primary ?? null}
						secondary={stats?.secondary ?? null}
					/>
				</button>
			</HoverCardTrigger>
			<HoverCardContent side="top" align="end" className="w-[280px]">
				<div className="flex flex-col gap-3 px-1 py-1">
					<div className="flex items-center justify-between">
						<div className="text-body font-semibold text-foreground">
							Usage Stats
						</div>
						<span
							className="text-muted-foreground"
							aria-label={agentType === "claude" ? "Claude" : "Codex"}
						>
							{agentType === "claude" ? (
								<ClaudeIcon className="size-[13px]" />
							) : (
								<OpenAIIcon className="size-[13px]" />
							)}
						</span>
					</div>
					{!stats && isLoading ? (
						<div className="text-small text-muted-foreground">Loading...</div>
					) : null}
					{stats?.primary || stats?.secondary || stats?.extraWindows.length ? (
						<div className="flex flex-col gap-2.5">
							{stats.primary ? <LimitRow window={stats.primary} /> : null}
							{stats.secondary ? <LimitRow window={stats.secondary} /> : null}
							{stats.extraWindows.map((entry) => (
								<LimitRow
									key={entry.id}
									window={{ ...entry.window, label: entry.title }}
								/>
							))}
						</div>
					) : null}
					{stats && stats.notes.length > 0 ? (
						<div className="flex flex-col gap-1.5 border-t border-border/40 pt-2.5">
							{stats.notes.map((note) => (
								<div
									key={note.label}
									className="flex items-center justify-between text-small"
								>
									<span className="text-muted-foreground">{note.label}</span>
									<span className="font-medium tabular-nums text-foreground">
										{note.value}
									</span>
								</div>
							))}
						</div>
					) : null}
				</div>
			</HoverCardContent>
		</HoverCard>
	);
}

// Geometry mirrors CodexBar's menu-bar IconRenderer so this indicator
// reads with the same visual weight users get from the system menu
// equivalent: 15 pt-wide track, 6 pt top bar, 4 pt bottom bar, 3 pt
// gap. The wrapper is `flex flex-col` rather than a fixed-size box, so
// the 28 pt button host (`size-7`) handles centering — the bars
// themselves stay tight against each other. Each fill colors
// independently by its own usedPercent tier.
const GLYPH_TRACK_WIDTH = 15;
const GLYPH_PRIMARY_HEIGHT = 6;
const GLYPH_SECONDARY_HEIGHT = 4;
const GLYPH_GAP = 3;

function UsageStatsGlyph({
	primary,
	secondary,
}: {
	primary: RateLimitWindowDisplay | null;
	secondary: RateLimitWindowDisplay | null;
}) {
	return (
		<div
			className="flex flex-col items-start justify-center"
			style={{ gap: GLYPH_GAP }}
			aria-hidden
		>
			<GlyphBar window={primary} height={GLYPH_PRIMARY_HEIGHT} />
			<GlyphBar window={secondary} height={GLYPH_SECONDARY_HEIGHT} />
		</div>
	);
}

function GlyphBar({
	window,
	height,
}: {
	window: RateLimitWindowDisplay | null;
	height: number;
}) {
	// Render the *remaining* fraction so the icon and popover read the
	// same way: longer bar = more headroom. Tier still tracks
	// `usedPercent` so a near-empty bar lights up amber/destructive.
	const used = window?.usedPercent ?? 0;
	const left = window ? window.leftPercent : 0;
	const tier = ringTier(used);
	const fillClass =
		tier === "danger"
			? "bg-destructive"
			: tier === "warning"
				? "bg-amber-500"
				: window
					? "bg-foreground/65"
					: "bg-foreground/35";
	return (
		<div
			className="overflow-hidden rounded-full bg-muted"
			style={{ width: GLYPH_TRACK_WIDTH, height }}
		>
			<div
				className={cn("h-full rounded-full transition-[width]", fillClass)}
				style={{ width: `${left}%` }}
			/>
		</div>
	);
}
