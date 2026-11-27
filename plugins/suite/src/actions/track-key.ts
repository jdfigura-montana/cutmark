//! Base for the track keys: press toggles Mute / Solo / Arm on the configured track,
//! and the key renders the track's name and flag from the app state
//! (spec: docs/audio-editor/1.3-StreamDeck.md "Keys" and "State published").

import { type DidReceiveSettingsEvent, type KeyAction, type WillAppearEvent } from "@elgato/streamdeck";

import type { AppConnection, AppState, TrackState } from "../connection";
import { CommandKeyAction, type CommandKeySettings } from "./command-key";

/** The settings the track keys read, as written by the property inspector. */
export type TrackKeySettings = { track?: number };

/** The highest track number the property inspector offers. */
const MAX_TRACK = 64;

/** Reads the 1-based track number from a settings value, clamped to 1..64, defaulting to 1. */
export function trackNumber(value: unknown): number {
	if (typeof value !== "number" || !Number.isFinite(value)) return 1;
	return Math.min(MAX_TRACK, Math.max(1, Math.trunc(value)));
}

/**
 * Base for the Track Mute / Solo / Arm keys: the title is the track's name from the
 * app state, the on state lights while the track's flag is set, and a track the app
 * does not have shows "No track N".
 */
export abstract class TrackKeyAction extends CommandKeyAction {
	/** The track number each visible key is configured for. */
	private readonly trackByAction = new WeakMap<KeyAction<CommandKeySettings>, number>();

	/** Remembers the key's track number and starts rendering. */
	override onWillAppear(ev: WillAppearEvent<CommandKeySettings>): void {
		if (ev.action.isKey()) this.trackByAction.set(ev.action, trackNumber(ev.payload.settings["track"]));
		super.onWillAppear(ev);
	}

	/** Re-reads the track number when the settings change and redraws the key. */
	override onDidReceiveSettings(ev: DidReceiveSettingsEvent<CommandKeySettings>): void {
		if (ev.action.isKey()) {
			this.trackByAction.set(ev.action, trackNumber(ev.payload.settings["track"]));
			this.render(ev.action);
		}
	}

	/** Sends the command with the configured track number. */
	protected override sendRunning(conn: AppConnection, settings: CommandKeySettings): void {
		conn.send(this.command, { track: trackNumber(settings["track"]) });
	}

	/** Renders the track's name and lit state, or "No track N". */
	protected override renderRunning(key: KeyAction<CommandKeySettings>, state: AppState): void {
		const n = this.trackByAction.get(key) ?? 1;
		const track = (state.tracks ?? []).find((candidate) => candidate.n === n);
		void key.setImage(undefined);
		if (!track) {
			void key.setState(0);
			void key.setTitle(`No track ${n}`);
			return;
		}
		void key.setState(this.isLit(track) ? 1 : 0);
		void key.setTitle(track.name);
	}

	/** Whether the key's on state is lit for this track. */
	protected abstract isLit(track: TrackState): boolean;
}