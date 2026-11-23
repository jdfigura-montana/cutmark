//! The Cutmark Audio key actions, one per row of the Keys table
//! (docs/audio-editor/1.3-StreamDeck.md "Keys"); track keys and the Audio
//! command action arrive in R4.

import { action, type KeyAction } from "@elgato/streamdeck";

import type { AppState } from "../connection";
import { CommandKeyAction, type CommandKeySettings } from "./command-key";

/** The key action type the subclasses render with. */
type Key = KeyAction<CommandKeySettings>;

/** Play key: play icon, stop icon while playing or recording. */
@action({ UUID: "com.cutmark.suite.audio.play_pause" })
export class PlayStopKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("play_pause", "play-stop");
	}

	protected override isOn(state: AppState): boolean {
		return state.transport === "playing" || state.transport === "recording";
	}
}

/** Pause key. */
@action({ UUID: "com.cutmark.suite.audio.pause" })
export class PauseKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("pause", "pause");
	}
}

/** Record key: red while recording, dimmed when recording is not allowed. */
@action({ UUID: "com.cutmark.suite.audio.record" })
export class RecordKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("record", "record");
	}

	protected override renderRunning(key: Key, state: AppState): void {
		void key.setImage(undefined);
		if (state.transport === "recording") {
			void key.setState(1);
			return;
		}
		void key.setState(0);
		if (state.can_record === false) void key.setImage(this.dimmedIcon);
	}
}

/** Jump to start key. */
@action({ UUID: "com.cutmark.suite.audio.jump_to_start" })
export class JumpToStartKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("jump_to_start", "jump-start");
	}
}

/** Jump to end key. */
@action({ UUID: "com.cutmark.suite.audio.jump_to_end" })
export class JumpToEndKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("jump_to_end", "jump-end");
	}
}

/** Add marker key. */
@action({ UUID: "com.cutmark.suite.audio.add_marker" })
export class AddMarkerKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("add_marker", "add-marker");
	}
}

/** Previous marker key. */
@action({ UUID: "com.cutmark.suite.audio.previous_marker" })
export class PreviousMarkerKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("previous_marker", "prev-marker");
	}
}

/** Next marker key. */
@action({ UUID: "com.cutmark.suite.audio.next_marker" })
export class NextMarkerKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("next_marker", "next-marker");
	}
}

/** Undo key: dimmed when there is nothing to undo. */
@action({ UUID: "com.cutmark.suite.audio.undo" })
export class UndoKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("undo", "undo");
	}

	protected override renderRunning(key: Key, state: AppState): void {
		void key.setState(0);
		void key.setImage(state.can_undo === false ? this.dimmedIcon : undefined);
	}
}

/** Redo key: dimmed when there is nothing to redo. */
@action({ UUID: "com.cutmark.suite.audio.redo" })
export class RedoKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("redo", "redo");
	}

	protected override renderRunning(key: Key, state: AppState): void {
		void key.setState(0);
		void key.setImage(state.can_redo === false ? this.dimmedIcon : undefined);
	}
}

/** Split at playhead key. */
@action({ UUID: "com.cutmark.suite.audio.split_at_playhead" })
export class SplitAtPlayheadKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("split_at_playhead", "split");
	}
}

/** Ripple toggle: on state in the accent blue. */
@action({ UUID: "com.cutmark.suite.audio.toggle_ripple" })
export class RippleKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("toggle_ripple", "ripple");
	}

	protected override isOn(state: AppState): boolean {
		return state.ripple === true;
	}
}

/** Snap toggle: on state in the accent blue. */
@action({ UUID: "com.cutmark.suite.audio.toggle_snap" })
export class SnapKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("toggle_snap", "snap");
	}

	protected override isOn(state: AppState): boolean {
		return state.snap === true;
	}
}

/** Save key. */
@action({ UUID: "com.cutmark.suite.audio.save" })
export class SaveKey extends CommandKeyAction {
	/** Builds the key. */
	constructor() {
		super("save", "save");
	}
}
