//! The CutMark Recorder track gain dial for Stream Deck+
//! (spec: docs/suite/recorder-stream-deck.md "Plugin: Stream Deck+ dial").

import { action, type DidReceiveSettingsEvent, type WillAppearEvent } from "@elgato/streamdeck";

import type { AppConnection, AppState } from "../connection";
import { DIM_COLOR, DialBase, NORMAL_COLOR, type Dial, type DialSettings } from "./dial";
import { trackNumber } from "./track-key";

/** The gain change of one rotation tick, in dB. */
const DB_PER_TICK = 0.5;

/** The level the strip's bar shows as empty, in dBFS; 0 dBFS fills it. */
const LEVEL_FLOOR_DBFS = -60;

/** One track of the state the Recorder publishes (spec "State"). */
type RecorderTrack = { index?: number; name?: string; muted?: boolean; gain_db?: number; peak_dbfs?: number };

/** What the strip shows; the keys are the items of `layouts/recorder-gain.json`. */
type Strip = { title: string; value: { value: string; color: string }; indicator: number };

/** The track's gain as a signed dB text. */
function gainText(db: number | undefined): string {
	const value = typeof db === "number" && Number.isFinite(db) ? db : 0;
	return `${value >= 0 ? "+" : ""}${value.toFixed(1)} dB`;
}

/** The level bar's fill for a peak, in whole percent; empty when there is no peak. */
function levelPercent(peakDbfs: number | undefined): number {
	if (typeof peakDbfs !== "number" || !Number.isFinite(peakDbfs)) return 0;
	return Math.round(Math.min(100, Math.max(0, (1 - peakDbfs / LEVEL_FLOOR_DBFS) * 100)));
}

/** The strip of the configured track, or "No track N". */
function stripFor(state: AppState, number: number): Strip {
	const tracks = (state.tracks ?? []) as unknown as RecorderTrack[];
	const track = tracks.find((candidate) => candidate.index === number);
	if (!track) return { title: `No track ${number}`, value: { value: "", color: NORMAL_COLOR }, indicator: 0 };
	const title = track.name ?? `Track ${number}`;
	if (track.muted === true) return { title, value: { value: "Muted", color: DIM_COLOR }, indicator: 0 };
	return { title, value: { value: gainText(track.gain_db), color: NORMAL_COLOR }, indicator: levelPercent(track.peak_dbfs) };
}

/** Track gain dial: rotate changes the track's gain, press mutes it; the strip shows its live level. */
@action({ UUID: "com.cutmark.suite.recorder.dial-track-gain" })
export class RecorderTrackGainDial extends DialBase {
	/** The track number each visible dial is configured for. */
	private readonly trackByDial = new WeakMap<Dial, number>();

	/** The strip each dial last drew, so a patch that changes nothing on it sends nothing. */
	private readonly drawn = new WeakMap<Dial, string>();

	/** Builds the dial. */
	constructor() {
		super(["track_gain", "track_mute"]);
	}

	/** Remembers the dial's track number and draws it afresh. */
	override onWillAppear(ev: WillAppearEvent<DialSettings>): void {
		if (ev.action.isDial()) {
			this.trackByDial.set(ev.action, trackNumber(ev.payload.settings["track"]));
			this.drawn.delete(ev.action);
		}
		super.onWillAppear(ev);
	}

	/** Re-reads the track number when the settings change and redraws the strip. */
	override onDidReceiveSettings(ev: DidReceiveSettingsEvent<DialSettings>): void {
		if (ev.action.isDial()) {
			this.trackByDial.set(ev.action, trackNumber(ev.payload.settings["track"]));
			this.render(ev.action);
		}
	}

	/** Sends the gain change of the coalesced ticks for the configured track. */
	protected override onRotated(conn: AppConnection, dial: Dial, ticks: number): void {
		conn.send("track_gain", { track: this.trackByDial.get(dial) ?? 1, delta_db: ticks * DB_PER_TICK });
	}

	/** Mutes or unmutes the configured track. */
	protected override onPressed(conn: AppConnection, dial: Dial): void {
		conn.send("track_mute", { track: this.trackByDial.get(dial) ?? 1 });
	}

	/** What rotating and pressing the dial do. */
	protected override triggers() {
		return { rotate: "Track gain", push: "Mute / unmute the track" };
	}

	/** Forgets the drawn strip while the base shows "Not running" or "Update". */
	protected override render(dial: Dial): void {
		if (!this.conn?.running || this.conn.updateMode) this.drawn.delete(dial);
		super.render(dial);
	}

	/** The strip: the track's name, its gain or "Muted", and its level. */
	protected override renderRunning(dial: Dial, state: AppState): void {
		const strip = stripFor(state, this.trackByDial.get(dial) ?? 1);
		const signature = JSON.stringify(strip);
		if (this.drawn.get(dial) === signature) return;
		this.drawn.set(dial, signature);
		void dial.setFeedback(strip);
	}
}
