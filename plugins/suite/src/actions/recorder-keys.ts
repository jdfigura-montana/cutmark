//! The CutMark Recorder key actions, one per row of the keys table
//! (spec: docs/suite/recorder-stream-deck.md "Plugin: Recorder keys").

import {
	action,
	type DidReceiveSettingsEvent,
	type KeyAction,
	type KeyDownEvent,
	type WillAppearEvent,
	type WillDisappearEvent,
} from "@elgato/streamdeck";

import type { AppConnection } from "../connection";
import { CommandKeyAction, type CommandKeySettings } from "./command-key";
import { trackNumber } from "./track-key";

/** The state the Recorder publishes (spec "State"), limited to what the keys render. */
type RecorderState = {
	phase?: "idle" | "countdown" | "recording" | "paused";
	elapsed_ms?: number;
	countdown_s?: number | null;
	replay_on?: boolean;
	mic_muted?: boolean;
	tracks?: { index?: number; name?: string; muted?: boolean }[];
};

/** What one key shows; `image` and `title` fall back to the manifest's when undefined. */
type Face = { state: 0 | 1; image?: string; title?: string };

/** The key action type the subclasses render with. */
type Key = KeyAction<CommandKeySettings>;

/** How long a key shows its flash image after a press. */
const FLASH_MS = 300;

/** The mark categories the app accepts; the first is the default. */
const MARK_CATEGORIES = ["note", "retake", "cut", "broll", "chapter"];

/** Reads the mark category from the key's settings, defaulting to note. */
function markCategory(value: unknown): string {
	return typeof value === "string" && MARK_CATEGORIES.includes(value) ? value : "note";
}

/** The app is idle; a state without a phase counts as idle. */
function isIdle(state: RecorderState): boolean {
	return (state.phase ?? "idle") === "idle";
}

/** Formats a take's elapsed time as mm:ss; the minutes keep counting past 59. */
function formatElapsed(ms: number): string {
	const seconds = Math.max(0, Math.floor(ms / 1000));
	const pad = (value: number) => String(value).padStart(2, "0");
	return `${pad(Math.floor(seconds / 60))}:${pad(seconds % 60)}`;
}

/**
 * Base for the Recorder keys. The Recorder's state changes many times a second
 * (elapsed time and track peaks), so a key is redrawn only when its face changes.
 */
abstract class RecorderKeyAction extends CommandKeyAction {
	/** The folder under `imgs/actions/` holding this key's icons. */
	protected readonly imageFolder: string;

	private readonly settingsByKey = new WeakMap<Key, CommandKeySettings>();
	private readonly drawn = new WeakMap<Key, string>();
	private readonly flashTimers = new Map<string, NodeJS.Timeout>();
	private readonly flashing = new WeakSet<Key>();

	constructor(command: string, iconFolder: string) {
		super(command, iconFolder);
		this.imageFolder = `imgs/actions/${iconFolder}`;
	}

	/** Remembers the key's settings and draws it afresh. */
	override onWillAppear(ev: WillAppearEvent<CommandKeySettings>): void {
		if (ev.action.isKey()) {
			this.settingsByKey.set(ev.action, ev.payload.settings);
			this.drawn.delete(ev.action);
		}
		super.onWillAppear(ev);
	}

	/** Ends a running flash when the key leaves the deck. */
	override onWillDisappear(ev: WillDisappearEvent<CommandKeySettings>): void {
		clearTimeout(this.flashTimers.get(ev.action.id));
		this.flashTimers.delete(ev.action.id);
		this.flashing.delete(ev.action as Key);
		super.onWillDisappear(ev);
	}

	/** Re-reads the settings and redraws the key. */
	override onDidReceiveSettings(ev: DidReceiveSettingsEvent<CommandKeySettings>): void {
		if (!ev.action.isKey()) return;
		this.settingsByKey.set(ev.action, ev.payload.settings);
		this.render(ev.action);
	}

	/** Sends the command or launches the app, then flashes the key if this press calls for it. */
	override async onKeyDown(ev: KeyDownEvent<CommandKeySettings>): Promise<void> {
		await super.onKeyDown(ev);
		const conn = this.conn;
		if (conn?.running && this.flashesOnPress(this.stateOf(conn))) this.flash(ev.action);
	}

	protected override render(key: Key): void {
		const conn = this.conn;
		if (!conn) return;
		let face: Face;
		if (conn.updateMode) {
			face = { state: 0, title: "Update" };
		} else if (!conn.running) {
			face = { state: 0, image: this.dimmedIcon, title: "Open" };
		} else {
			face = this.face(this.stateOf(conn), this.settingsByKey.get(key) ?? {});
			if (this.flashing.has(key)) face = { ...face, image: `${this.imageFolder}/flash` };
		}
		const signature = JSON.stringify(face);
		if (this.drawn.get(key) === signature) return;
		this.drawn.set(key, signature);
		void key.setState(face.state);
		void key.setImage(face.image);
		void key.setTitle(face.title);
	}

	/** What the key shows while the app runs. */
	protected face(_state: RecorderState, _settings: CommandKeySettings): Face {
		return { state: 0 };
	}

	/** Whether a press in this state flashes the key. */
	protected flashesOnPress(_state: RecorderState): boolean {
		return false;
	}

	private stateOf(conn: AppConnection): RecorderState {
		return (conn.state ?? {}) as unknown as RecorderState;
	}

	private flash(key: Key): void {
		clearTimeout(this.flashTimers.get(key.id));
		this.flashing.add(key);
		this.render(key);
		this.flashTimers.set(
			key.id,
			setTimeout(() => {
				this.flashTimers.delete(key.id);
				this.flashing.delete(key);
				this.render(key);
			}, FLASH_MS),
		);
	}
}

/** Record / Stop: red with the elapsed time during a take. */
@action({ UUID: "com.cutmark.suite.recorder.start-stop" })
export class RecorderRecordKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("start_stop", "recorder-record");
	}

	protected override face(state: RecorderState): Face {
		if (isIdle(state)) return { state: 0 };
		// A Recorder that publishes no countdown number gets "...".
		if (state.phase === "countdown") return { state: 0, title: String(state.countdown_s ?? "...") };
		return { state: 1, title: formatElapsed(state.elapsed_ms ?? 0) };
	}
}

/** Pause / Resume: lit while paused, dimmed while idle. */
@action({ UUID: "com.cutmark.suite.recorder.pause-resume" })
export class RecorderPauseKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("pause_resume", "recorder-pause");
	}

	protected override face(state: RecorderState): Face {
		if (isIdle(state)) return { state: 0, image: this.dimmedIcon };
		return { state: state.phase === "paused" ? 1 : 0 };
	}
}

/** Mark: shows the configured category's letter and flashes on press. */
@action({ UUID: "com.cutmark.suite.recorder.mark" })
export class RecorderMarkKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("mark", "recorder-mark");
	}

	protected override sendRunning(conn: AppConnection, settings: CommandKeySettings): void {
		conn.send(this.command, { category: markCategory(settings["category"]) });
	}

	protected override face(_state: RecorderState, settings: CommandKeySettings): Face {
		return { state: 0, image: `${this.imageFolder}/${markCategory(settings["category"])}` };
	}

	protected override flashesOnPress(): boolean {
		return true;
	}
}

/** Mark with label: flashes on press. */
@action({ UUID: "com.cutmark.suite.recorder.mark-label" })
export class RecorderMarkLabelKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("mark_label", "recorder-mark-label");
	}

	protected override flashesOnPress(): boolean {
		return true;
	}
}

/** Highlight last 30 s: flashes on press, dimmed while idle. */
@action({ UUID: "com.cutmark.suite.recorder.highlight-last" })
export class RecorderHighlightKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("highlight_last", "recorder-highlight");
	}

	protected override face(state: RecorderState): Face {
		return { state: 0, image: isIdle(state) ? this.dimmedIcon : undefined };
	}

	protected override flashesOnPress(state: RecorderState): boolean {
		return !isIdle(state);
	}
}

/** Mute mic: lit with the muted icon while muted. */
@action({ UUID: "com.cutmark.suite.recorder.mute-mic" })
export class RecorderMuteMicKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("mute_mic", "recorder-mute-mic");
	}

	protected override face(state: RecorderState): Face {
		return { state: state.mic_muted === true ? 1 : 0 };
	}
}

/** Replay buffer: lit while on. */
@action({ UUID: "com.cutmark.suite.recorder.replay-toggle" })
export class RecorderReplayKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("replay_toggle", "recorder-replay");
	}

	protected override face(state: RecorderState): Face {
		return { state: state.replay_on === true ? 1 : 0 };
	}
}

/** Save replay: dimmed while the buffer is off, flashes on a save press. */
@action({ UUID: "com.cutmark.suite.recorder.replay-save" })
export class RecorderSaveReplayKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("replay_save", "recorder-save-replay");
	}

	protected override face(state: RecorderState): Face {
		return { state: 0, image: state.replay_on === true ? undefined : this.dimmedIcon };
	}

	protected override flashesOnPress(state: RecorderState): boolean {
		return state.replay_on === true;
	}
}

/** Track mute: the track's name, lit while muted, or "No track N". */
@action({ UUID: "com.cutmark.suite.recorder.track-mute" })
export class RecorderTrackMuteKey extends RecorderKeyAction {
	/** Builds the key. */
	constructor() {
		super("track_mute", "recorder-track-mute");
	}

	protected override sendRunning(conn: AppConnection, settings: CommandKeySettings): void {
		conn.send(this.command, { track: trackNumber(settings["track"]) });
	}

	protected override face(state: RecorderState, settings: CommandKeySettings): Face {
		const number = trackNumber(settings["track"]);
		const track = (state.tracks ?? []).find((candidate) => candidate.index === number);
		if (!track) return { state: 0, title: `No track ${number}` };
		return { state: track.muted === true ? 1 : 0, title: track.name };
	}
}
