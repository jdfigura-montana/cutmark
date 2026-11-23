//! Base class for the Cutmark key actions: press sends one remote command and the
//! key renders from the app's state (spec: Remote Control "Plugin", 1.3-StreamDeck "Keys").

import { type KeyAction, type KeyDownEvent, type WillAppearEvent, type WillDisappearEvent, SingletonAction } from "@elgato/streamdeck";
import type { JsonObject } from "@elgato/utils";

import type { AppConnection, AppState, RemoteError } from "../connection";

/** Settings of a Cutmark key, read with guards; the R3 keys carry none (track keys add `track`, the Audio command adds `command`). */
export type CommandKeySettings = JsonObject;

/**
 * Base for keys that send one remote command. While the app is not running the key
 * shows the dimmed icon with the title "Open" and a press launches the app; while it
 * runs the key renders from the state and a press sends the command.
 */
export abstract class CommandKeyAction extends SingletonAction<CommandKeySettings> {
	/** The remote command id this key sends. */
	protected readonly command: string;

	/** Path (without extension) of the dimmed icon shown when the app is not running or blocked. */
	protected readonly dimmedIcon: string;

	/** The app connection, set by `use`. */
	protected conn: AppConnection | undefined;

	private readonly keys = new Set<KeyAction<CommandKeySettings>>();

	/**
	 * Creates the action.
	 * @param command The remote command id (an entry of the app's shortcut table).
	 * @param iconFolder The folder under `imgs/actions/` holding this key's icons.
	 */
	constructor(command: string, iconFolder: string) {
		super();
		this.command = command;
		this.dimmedIcon = `imgs/actions/${iconFolder}/dimmed`;
	}

	/** Wires the action to the app's connection; call once for each action at startup. */
	use(conn: AppConnection): void {
		this.conn = conn;
		conn.onChange(() => {
			for (const key of [...this.keys]) this.render(key);
		});
		conn.onError((error: RemoteError) => {
			if (!this.isOwnError(error)) return;
			for (const key of [...this.keys]) void key.showAlert();
		});
	}

	/** Starts rendering the key while it is visible on the deck. */
	override onWillAppear(ev: WillAppearEvent<CommandKeySettings>): void {
		if (!ev.action.isKey()) return;
		this.keys.add(ev.action);
		this.render(ev.action);
	}

	/** Stops rendering the key when it leaves the deck. */
	override onWillDisappear(ev: WillDisappearEvent<CommandKeySettings>): void {
		this.keys.delete(ev.action as KeyAction<CommandKeySettings>);
	}

	/** Sends the command, or launches the app when it is not running. */
	override async onKeyDown(ev: KeyDownEvent<CommandKeySettings>): Promise<void> {
		const conn = this.conn;
		if (!conn || conn.updateMode) return;
		if (!conn.running) {
			conn.launch();
			return;
		}
		this.sendRunning(conn, ev.payload.settings);
	}

	/** Sends the key's command while the app runs; subclasses override this to read settings. */
	protected sendRunning(conn: AppConnection, _settings: CommandKeySettings): void {
		conn.send(this.command);
	}

	/** Whether an `error` message belongs to this key's command; subclasses override for dynamic commands. */
	protected isOwnError(error: RemoteError): boolean {
		return error.id === this.command;
	}

	/** Redraws one key from the connection state. */
	protected render(key: KeyAction<CommandKeySettings>): void {
		const conn = this.conn;
		if (!conn) return;
		if (conn.updateMode) {
			void key.setState(0);
			void key.setImage(undefined);
			void key.setTitle("Update");
			return;
		}
		if (!conn.running) {
			void key.setState(0);
			void key.setImage(this.dimmedIcon);
			void key.setTitle("Open");
			return;
		}
		void key.setTitle(undefined);
		this.renderRunning(key, conn.state ?? {});
	}

	/** Renders the key while the app runs; subclasses override this for stateful keys. */
	protected renderRunning(key: KeyAction<CommandKeySettings>, state: AppState): void {
		void key.setImage(undefined);
		void key.setState(this.isOn(state) ? 1 : 0);
	}

	/** Whether the key's second (on) state is active; override for toggle keys. */
	protected isOn(state: AppState): boolean {
		return false;
	}
}
