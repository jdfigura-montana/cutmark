//! The Audio command key: press runs the command chosen in the property inspector,
//! picked from the command list the app sent with `welcome`
//! (spec: docs/audio-editor/1.3-StreamDeck.md "Keys").

import streamDeck, { type DidReceiveSettingsEvent, type KeyAction, type SendToPluginEvent, type WillAppearEvent } from "@elgato/streamdeck";
import type { JsonValue } from "@elgato/utils";

import type { AppConnection, AppState, RemoteError } from "../connection";
import { CommandKeyAction, type CommandKeySettings } from "./command-key";

/** The settings the Audio command key reads, as written by the property inspector. */
export type AudioCommandSettings = { command?: string };

/** The option the dropdown falls back to until the app's command list is available. */
const NO_COMMANDS = "Open the audio editor to load commands";

/** The `datasource` event name the property inspector's dropdown asks for. */
const DATASOURCE_EVENT = "getCommands";

/** One option of the property inspector's dropdown. */
type CommandOption = { label: string; value: string; disabled?: boolean };

/** Reads the chosen command id from a settings value; "" when none is chosen. */
function chosenCommand(value: unknown): string {
	return typeof value === "string" ? value : "";
}

/**
 * The Audio command key: the title is the chosen command's label, a press sends that
 * command, and the property inspector's dropdown is filled from the app's command list.
 */
export class AudioCommandKey extends CommandKeyAction {
	/** The command each visible key is configured for. */
	private readonly commandByAction = new WeakMap<KeyAction<CommandKeySettings>, string>();

	/** The command ids of the visible keys, for matching `error` messages. */
	private readonly commands = new Set<string>();

	/** Builds the key. */
	constructor() {
		super("", "audio-command");
	}

	/** Remembers the key's command and starts rendering. */
	override onWillAppear(ev: WillAppearEvent<CommandKeySettings>): void {
		if (ev.action.isKey()) this.remember(ev.action, chosenCommand(ev.payload.settings["command"]));
		super.onWillAppear(ev);
	}

	/** Re-reads the command when the settings change and redraws the key. */
	override onDidReceiveSettings(ev: DidReceiveSettingsEvent<CommandKeySettings>): void {
		if (ev.action.isKey()) {
			this.remember(ev.action, chosenCommand(ev.payload.settings["command"]));
			this.render(ev.action);
		}
	}

	/** Sends the chosen command; a press with no command chosen does nothing. */
	protected override sendRunning(conn: AppConnection, settings: CommandKeySettings): void {
		const command = chosenCommand(settings["command"]);
		if (command) conn.send(command);
	}

	/** Alerts the key only for errors about a command one of its keys sends. */
	protected override isOwnError(error: RemoteError): boolean {
		return error.id !== undefined && this.commands.has(error.id);
	}

	/** Renders the chosen command's label as the title. */
	protected override renderRunning(key: KeyAction<CommandKeySettings>, _state: AppState): void {
		void key.setImage(undefined);
		void key.setState(0);
		const command = this.commandByAction.get(key) ?? "";
		const info = command ? this.conn?.commands.find((candidate) => candidate.id === command) : undefined;
		const title = info ? info.label : command || undefined;
		void key.setTitle(title);
	}

	/** Fills the property inspector's dropdown from the app's command list. */
	override onSendToPlugin(ev: SendToPluginEvent<JsonValue, CommandKeySettings>): void {
		const payload = ev.payload;
		if (!payload || typeof payload !== "object" || Array.isArray(payload)) return;
		if ((payload as { event?: unknown }).event !== DATASOURCE_EVENT) return;
		void streamDeck.ui.sendToPropertyInspector({ event: DATASOURCE_EVENT, items: this.options() });
	}

	/** The dropdown's options: the app's commands, or the fallback option. */
	private options(): CommandOption[] {
		const conn = this.conn;
		if (conn && conn.running && conn.commands.length > 0) {
			return conn.commands.map((command) => ({ label: command.label, value: command.id }));
		}
		return [{ label: NO_COMMANDS, value: "", disabled: true }];
	}

	/** Stores a key's command and keeps the error-matching set in sync. */
	private remember(key: KeyAction<CommandKeySettings>, command: string): void {
		this.commandByAction.set(key, command);
		if (command) this.commands.add(command);
	}
}