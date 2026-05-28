import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { resolveSystemAgentBinary } from "@/lib/api";
import { extractError } from "@/lib/errors";
import { SettingsRow } from "../components/settings-row";

type Props = {
	value: string;
	onChange: (value: string) => void;
};

export function CodexBinarySettingsRow({ value, onChange }: Props) {
	const [draft, setDraft] = useState(value);
	const [resolvingSystemPath, setResolvingSystemPath] = useState(false);

	useEffect(() => {
		setDraft(value);
	}, [value]);

	function commit(nextValue = draft) {
		const next = nextValue.trim();
		setDraft(next);
		if (next !== value) onChange(next);
	}

	async function pickExecutable() {
		const selection = await openDialog({
			title: "Select Codex executable",
			multiple: false,
			directory: false,
		});
		if (typeof selection !== "string") return;
		setDraft(selection);
		onChange(selection);
	}

	function setPreset(next: string) {
		setDraft(next);
		if (next !== value) onChange(next);
	}

	async function handleUseSystemCodex() {
		setResolvingSystemPath(true);
		try {
			setPreset(await resolveSystemAgentBinary("codex"));
		} catch (error) {
			const { message } = extractError(error, "Unable to resolve Codex path");
			toast.error("Codex was not found on PATH", {
				description: message,
			});
		} finally {
			setResolvingSystemPath(false);
		}
	}

	return (
		<SettingsRow
			title="Codex executable path"
			description="Override the bundled Codex executable. Leave empty to use the bundled version."
			align="start"
			className="gap-8"
		>
			<div className="flex w-[360px] flex-col gap-2">
				<div className="flex items-center gap-2">
					<Input
						value={draft}
						onBlur={() => commit()}
						onChange={(event) => setDraft(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter") {
								event.currentTarget.blur();
							}
						}}
						placeholder="/usr/local/bin/codex"
						className="h-8 min-w-0 flex-1 border-border/50 bg-muted/20 font-mono text-ui"
					/>
					<Button
						type="button"
						variant="outline"
						size="icon"
						aria-label="Choose Codex executable"
						className="size-8 shrink-0"
						onClick={() => void pickExecutable()}
					>
						<FolderOpen className="size-4" strokeWidth={1.8} />
					</Button>
				</div>
				<div className="flex items-center gap-2">
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={resolvingSystemPath}
						onClick={() => void handleUseSystemCodex()}
					>
						{resolvingSystemPath
							? "Resolving system Codex..."
							: "Use system Codex (from PATH)"}
					</Button>
					{value ? (
						<Button
							type="button"
							variant="ghost"
							size="sm"
							onClick={() => setPreset("")}
						>
							Use bundled Codex
						</Button>
					) : null}
				</div>
			</div>
		</SettingsRow>
	);
}
