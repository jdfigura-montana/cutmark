//! The Cutmark Audio dial actions for Stream Deck+ (docs/audio-editor/1.3-StreamDeck.md "Dials"):
//! Zoom, Jog, Track volume and Markers, each with rotate, press and touch tap per the table.

import { action, type DidReceiveSettingsEvent, type WillAppearEvent } from "@elgato/streamdeck";

import type { AppConnection, AppState, TrackState } from "../connection";
import { DIM_COLOR, DialBase, NORMAL_COLOR, type Dial, type DialSettings } from "./dial";
import { trackNumber } from "./track-key";

/** The volume range the track header's fader shows, for the strip's bar. */
const VOLUME_RANGE: Readonly<{ min: number; max: number }> = { min: -60, max: 6 };

/** The playhead as mm:ss.mmm, from the state's `playhead_ms`. */
function playheadTimecode(state: AppState): string {
	const ms = state.playhead_ms;
	if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return "00:00.000";
	const whole = Math.floor(ms);
	const minutes = Math.floor(whole / 60000);
	const seconds = Math.floor(whole / 1000) % 60;
	const millis = whole % 1000;
	return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${String(millis).padStart(3, "0")}`;
}

/** The zoom bar's fill from the state's `zoom_label` ("100%" is the fit zoom), 0..100; 0 when unknown. */
function zoomFraction(zoomLabel: string | undefined): number {
	const match = typeof zoomLabel === "string" ? /([\d.]+)\s*%$/.exec(zoomLabel.trim()) : null;
	if (!match) return 0;
	return Math.min(100, Math.max(0, Number(match[1])));
}

/** The track's volume as a signed dB text. */
function dbText(db: number | undefined): string {
	const value = typeof db === "number" && Number.isFinite(db) ? db : 0;
	return `${value >= 0 ? "+" : ""}${value.toFixed(1)} dB`;
}

/** The track volume bar's fill over the fader's range, 0..100. */
function volumeFraction(db: number | undefined): number {
	const value = typeof db === "number" && Number.isFinite(db) ? db : 0;
	return Math.min(100, Math.max(0, ((value - VOLUME_RANGE.min) / (VOLUME_RANGE.max - VOLUME_RANGE.min)) * 100));
}

/** The track of the state's `tracks` with the configured 1-based number, if the app has it. */
function findTrack(state: AppState, n: number): TrackState | undefined {
	return (state.tracks ?? []).find((candidate) => candidate.n === n);
}

/** Zoom dial: rotate zooms the view, press fits the project, strip shows the zoom percent and bar. */
@action({ UUID: "com.cutmark.suite.audio.dial_zoom" })
export class ZoomDial extends DialBase {
	/** Builds the dial. */
	constructor() {
		super(["zoom.step", "zoom.fit"]);
	}

	/** Sends the zoom step command with the coalesced ticks. */
	protected override onRotated(conn: AppConnection, _dial: Dial, ticks: number): void {
		conn.send("zoom.step", { ticks });
	}

	/** Fits the project to the window. */
	protected override onPressed(conn: AppConnection, _dial: Dial): void {
		conn.send("zoom.fit");
	}

	/** What rotating and pressing the dial do. */
	protected override triggers() {
		return { rotate: "Zoom in and out", push: "Zoom to fit" };
	}

	/** The strip: "Zoom", the app's zoom label and the percent as a bar. */
	protected override renderRunning(dial: Dial, state: AppState): void {
		void dial.setFeedback({ title: "Zoom", value: state.zoom_label ?? "", bar: zoomFraction(state.zoom_label) });
	}
}

/** Jog dial: rotate jogs the playhead, press toggles fine / coarse, touch tap plays or stops. */
@action({ UUID: "com.cutmark.suite.audio.dial_jog" })
export class JogDial extends DialBase {
	/** The jog size each visible dial is in. */
	private readonly sizeByDial = new WeakMap<Dial, "fine" | "coarse">();

	/** Builds the dial. */
	constructor() {
		super(["jog.step", "play_pause"]);
	}

	/** The jog size of a dial, "fine" until the user presses it. */
	private size(dial: Dial): "fine" | "coarse" {
		return this.sizeByDial.get(dial) ?? "fine";
	}

	/** Sends the jog step command with the coalesced ticks and the dial's size. */
	protected override onRotated(conn: AppConnection, dial: Dial, ticks: number): void {
		conn.send("jog.step", { ticks, size: this.size(dial) });
	}

	/** Toggles the dial's jog size between fine (10 ms) and coarse (1 s). */
	protected override onPressed(conn: AppConnection, dial: Dial): void {
		this.sizeByDial.set(dial, this.size(dial) === "fine" ? "coarse" : "fine");
		this.renderRunning(dial, conn.state ?? {});
	}

	/** Plays or stops. */
	protected override onTouched(conn: AppConnection, _dial: Dial): void {
		conn.send("play_pause");
	}

	/** What rotating, pressing and touching the dial do. */
	protected override triggers() {
		return { rotate: "Jog the playhead", push: "Toggle fine / coarse", touch: "Play or stop" };
	}

	/** The strip: the playhead as mm:ss.mmm and the jog size. */
	protected override renderRunning(dial: Dial, state: AppState): void {
		void dial.setFeedback({ title: playheadTimecode(state), value: this.size(dial) === "fine" ? "Fine" : "Coarse" });
	}
}

/** Track volume dial: rotate steps the track's volume, press mutes it; the strip dims while muted. */
@action({ UUID: "com.cutmark.suite.audio.dial_track_volume" })
export class TrackVolumeDial extends DialBase {
	/** The track number each visible dial is configured for. */
	private readonly trackByDial = new WeakMap<Dial, number>();

	/** Builds the dial. */
	constructor() {
		super(["track.volume.step", "track.mute"]);
	}

	/** Remembers the dial's track number and starts rendering. */
	override onWillAppear(ev: WillAppearEvent<DialSettings>): void {
		if (ev.action.isDial()) this.trackByDial.set(ev.action, trackNumber(ev.payload.settings["track"]));
		super.onWillAppear(ev);
	}

	/** Re-reads the track number when the settings change and redraws the strip. */
	override onDidReceiveSettings(ev: DidReceiveSettingsEvent<DialSettings>): void {
		if (ev.action.isDial()) {
			this.trackByDial.set(ev.action, trackNumber(ev.payload.settings["track"]));
			this.render(ev.action);
		}
	}

	/** Sends the volume step command with the coalesced ticks and the configured track. */
	protected override onRotated(conn: AppConnection, dial: Dial, ticks: number): void {
		conn.send("track.volume.step", { ticks, track: this.trackByDial.get(dial) ?? 1 });
	}

	/** Mutes or unmutes the configured track. */
	protected override onPressed(conn: AppConnection, dial: Dial): void {
		conn.send("track.mute", { track: this.trackByDial.get(dial) ?? 1 });
	}

	/** What rotating and pressing the dial do. */
	protected override triggers() {
		return { rotate: "Track volume", push: "Mute / unmute the track" };
	}

	/** The strip: the track's name, its volume in dB and a bar; grey while the track is muted. */
	protected override renderRunning(dial: Dial, state: AppState): void {
		const n = this.trackByDial.get(dial) ?? 1;
		const track = findTrack(state, n);
		if (!track) {
			void dial.setFeedback({ title: `No track ${n}`, value: "", bar: 0 });
			return;
		}
		const muted = track.mute === true;
		const color = muted ? DIM_COLOR : NORMAL_COLOR;
		void dial.setFeedback({
			title: track.name ?? `Track ${n}`,
			value: { value: dbText(track.volume_db), color },
			bar: { value: volumeFraction(track.volume_db), bar_fill_c: color },
		});
	}
}

/** Markers dial: rotate jumps to the previous / next marker, press adds one at the playhead. */
@action({ UUID: "com.cutmark.suite.audio.dial_markers" })
export class MarkersDial extends DialBase {
	/** Builds the dial. */
	constructor() {
		super(["marker.step", "add_marker"]);
	}

	/** Sends the marker step command with the coalesced ticks. */
	protected override onRotated(conn: AppConnection, _dial: Dial, ticks: number): void {
		conn.send("marker.step", { ticks });
	}

	/** Adds a marker at the playhead. */
	protected override onPressed(conn: AppConnection, _dial: Dial): void {
		conn.send("add_marker");
	}

	/** What rotating and pressing the dial do. */
	protected override triggers() {
		return { rotate: "Previous / next marker", push: "Add marker at the playhead" };
	}

	/** The strip: "Markers" and the playhead as mm:ss.mmm. */
	protected override renderRunning(dial: Dial, state: AppState): void {
		void dial.setFeedback({ title: "Markers", value: playheadTimecode(state) });
	}
}
