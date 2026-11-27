//! Base class for the Cutmark dial actions: rotation ticks are coalesced into one command
//! per 50 ms, a press sends the dial's command (or launches the app when it is down), and
//! the touch strip renders from the app state (spec: docs/audio-editor/1.3-StreamDeck.md "Dials").

import {
	SingletonAction,
	type DialAction,
	type DialDownEvent,
	type DialRotateEvent,
	type TouchTapEvent,
	type TriggerDescriptionOptions,
	type WillAppearEvent,
	type WillDisappearEvent,
	type DidReceiveSettingsEvent,
} from "@elgato/streamdeck";
import type { JsonObject } from "@elgato/utils";

import type { AppConnection, AppState, RemoteError } from "../connection";

/** Settings of a Cutmark dial; the track volume dial adds `track`. */
export type DialSettings = JsonObject;

/** The dial action type the subclasses render with. */
export type Dial = DialAction<DialSettings>;

/** Rotation ticks are coalesced into one command per this many milliseconds (spec "Dials"). */
const COALESCE_MS = 50;

/** The strip color of text and bar while the track is muted. */
export const DIM_COLOR = "#7f7f7f";

/** The strip color of text and bar while the track is not muted. */
export const NORMAL_COLOR = "#ffffff";

/** A dial's pending rotation: the ticks seen since the window opened, and the flush timer. */
type PendingRotate = { ticks: number; timer: NodeJS.Timeout | undefined };

/**
 * Base for the Stream Deck+ dials. The touch strip renders from the app state through
 * `setFeedback`; while the app is not running it shows "Not running" and a press launches
 * the app, while it is in "Update" mode it shows "Update" and nothing is sent.
 */
export abstract class DialBase extends SingletonAction<DialSettings> {
	/** The app connection, set by `use`. */
	protected conn: AppConnection | undefined;

	/** The remote command ids this dial sends, for matching `error` messages. */
	private readonly commandIds: readonly string[];

	/** The dials currently visible on the deck. */
	private readonly dials = new Set<Dial>();

	/** The rotation ticks not sent yet, per dial. */
	private readonly pending = new WeakMap<Dial, PendingRotate>();

	/**
	 * Creates the dial.
	 * @param commandIds The remote command ids this dial sends (rotation, press, touch).
	 */
	constructor(commandIds: readonly string[]) {
		super();
		this.commandIds = commandIds;
	}

	/** Wires the dial to the app's connection; call once for each dial at startup. */
	use(conn: AppConnection): void {
		this.conn = conn;
		conn.onChange(() => {
			for (const dial of [...this.dials]) this.render(dial);
		});
		conn.onError((error: RemoteError) => {
			if (error.id === undefined || !this.commandIds.includes(error.id)) return;
			for (const dial of [...this.dials]) void dial.showAlert();
		});
	}

	/** Remembers the dial, sets its trigger descriptions and starts rendering. */
	override onWillAppear(ev: WillAppearEvent<DialSettings>): void {
		if (!ev.action.isDial()) return;
		this.dials.add(ev.action);
		void ev.action.setTriggerDescription(this.triggers());
		this.render(ev.action);
	}

	/** Forgets the dial and drops its unsent ticks when it leaves the deck. */
	override onWillDisappear(ev: WillDisappearEvent<DialSettings>): void {
		const dial = ev.action as Dial;
		this.dials.delete(dial);
		const pending = this.pending.get(dial);
		if (pending?.timer) clearTimeout(pending.timer);
		this.pending.delete(dial);
	}

	/** Re-renders the dial when its settings change. */
	override onDidReceiveSettings(ev: DidReceiveSettingsEvent<DialSettings>): void {
		if (ev.action.isDial()) this.render(ev.action);
	}

	/** Sends the press command, or launches the app when it is not running. */
	override onDialDown(ev: DialDownEvent<DialSettings>): void {
		const conn = this.conn;
		if (!conn || conn.updateMode) return;
		if (!conn.running) {
			conn.launch();
			return;
		}
		this.onPressed(conn, ev.action);
	}

	/** Sends the touch tap command while the app runs; the base dials have no touch action. */
	override onTouchTap(ev: TouchTapEvent<DialSettings>): void {
		const conn = this.conn;
		if (!conn || conn.updateMode || !conn.running) return;
		this.onTouched(conn, ev.action);
	}

	/** Sends the dial's command for a press while the app runs. */
	protected abstract onPressed(conn: AppConnection, dial: Dial): void;

	/** Sends the dial's command for the coalesced rotation ticks while the app runs. */
	protected abstract onRotated(conn: AppConnection, dial: Dial, ticks: number): void;

	/** The command for a touch tap while the app runs; the base dials have none. */
	protected onTouched(_conn: AppConnection, _dial: Dial): void {
		return;
	}

	/** The dial's trigger descriptions (rotate, push, touch). */
	protected abstract triggers(): TriggerDescriptionOptions;

	/** Renders the dial's touch strip while the app runs. */
	protected abstract renderRunning(dial: Dial, state: AppState): void;

	/** Renders the strip: "Update" in update mode, "Not running" while the app is down. */
	protected render(dial: Dial): void {
		const conn = this.conn;
		if (!conn) return;
		if (conn.updateMode) {
			void dial.setFeedback({ title: "Update", value: "", indicator: 0 });
			return;
		}
		if (!conn.running) {
			void dial.setFeedback({ title: "Not running", value: "", indicator: 0 });
			return;
		}
		this.renderRunning(dial, conn.state ?? {});
	}

	/** Sends the ticks accumulated over the last 50 ms window as one command. */
	private flush(conn: AppConnection, dial: Dial): void {
		const pending = this.pending.get(dial);
		if (!pending) return;
		this.pending.delete(dial);
		if (pending.ticks !== 0) this.onRotated(conn, dial, pending.ticks);
	}

	/** Coalesces the rotation ticks: one command per 50 ms, carrying the summed ticks. */
	override onDialRotate(ev: DialRotateEvent<DialSettings>): void {
		const conn = this.conn;
		if (!conn || conn.updateMode || !conn.running) return;
		const dial = ev.action;
		const pending = this.pending.get(dial) ?? { ticks: 0, timer: undefined };
		pending.ticks += ev.payload.ticks;
		if (pending.timer) return;
		pending.timer = setTimeout(() => this.flush(conn, dial), COALESCE_MS);
		this.pending.set(dial, pending);
	}
}
