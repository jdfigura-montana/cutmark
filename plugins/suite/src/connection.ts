//! The plugin's connection to one Cutmark app over the local remote-control
//! WebSocket (spec: Remote Control "Protocol v1" and "Plugin").

import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import streamDeck from "@elgato/streamdeck";
import WebSocket, { type RawData } from "ws";

/** The protocol major version the plugin speaks; a mismatch puts the keys in "Update" mode. */
export const PROTOCOL = 1;

/** How often the plugin retries a connection while the window is open. */
const RETRY_MS = 2000;

/** How long the plugin keeps retrying after a launch before waiting for the next launch event. */
const RETRY_WINDOW_MS = 30000;

/** The part of the app state the plugin renders (docs/audio-editor/1.3-StreamDeck.md "State published"). */
export type AppState = {
	project?: string | null;
	transport?: "stopped" | "playing" | "paused" | "recording";
	playhead_ms?: number;
	can_record?: boolean;
	can_undo?: boolean;
	can_redo?: boolean;
	ripple?: boolean;
	snap?: boolean;
	zoom_label?: string;

	/** The project's tracks, lowest number first. */
	tracks?: TrackState[];
};

/** A command the app accepts, as listed by the `welcome` message. */
export type CommandInfo = { id: string; label: string };

/** One track, from the state's `tracks` array (docs/audio-editor/1.3-StreamDeck.md "State published"). */
export type TrackState = {
	/** The 1-based track number. */
	n?: number;
	/** The track's name. */
	name?: string;
	/** Whether the track is muted. */
	mute?: boolean;
	/** Whether the track is soloed. */
	solo?: boolean;
	/** Whether the track is record-armed. */
	arm?: boolean;
	/** The track's volume in dB. */
	volume_db?: number;
};

/** An `error` message from the app, optionally about one command. */
export type RemoteError = { code: string; message: string; id?: string };

/** The `<app>.json` discovery file the running app writes. */
type Discovery = { pid: number; port: number; token: string };

/** The persistent `apps.json` launch table. */
type AppsFile = Record<string, { exe?: string; version?: string }>;

/** True when a process with this pid exists (EPERM means it exists but is not ours to signal). */
function pidRunning(pid: number): boolean {
	try {
		process.kill(pid, 0);
		return true;
	} catch (err) {
		return (err as NodeJS.ErrnoException).code === "EPERM";
	}
}

/** Reads the `<app>.json` discovery file; `undefined` when missing or malformed. */
function readDiscovery(dir: string, app: string): Discovery | undefined {
	let text: string;
	try {
		text = readFileSync(join(dir, `${app}.json`), "utf8");
	} catch {
		return undefined;
	}
	let value: unknown;
	try {
		value = JSON.parse(text);
	} catch {
		return undefined;
	}
	if (!value || typeof value !== "object") return undefined;
	const file = value as Record<string, unknown>;
	const pid = file["pid"];
	const port = file["port"];
	const token = file["token"];
	if (typeof pid !== "number" || typeof port !== "number" || typeof token !== "string") return undefined;
	return { pid, port, token };
}

/** Builds the command list from the `welcome` message; labels fall back to the id. */
function readCommands(value: unknown): CommandInfo[] {
	if (!Array.isArray(value)) return [];
	const commands: CommandInfo[] = [];
	for (const entry of value) {
		if (!entry || typeof entry !== "object") continue;
		const id = (entry as Record<string, unknown>)["id"];
		const label = (entry as Record<string, unknown>)["label"];
		if (typeof id !== "string") continue;
		commands.push({ id, label: typeof label === "string" ? label : id });
	}
	return commands;
}

/**
 * One app's connection: discovery, handshake, retries, state and errors.
 * The plugin creates one per app and key actions render from it.
 */
export class AppConnection {
	/** True while the app is connected and speaks our protocol major. */
	running = false;

	/** True when the app answered with a different protocol major; keys show "Update" and send nothing. */
	updateMode = false;

	/** Last known app state: the full snapshot with patches merged in. */
	state: AppState | undefined;

	/** The commands the app accepts, from `welcome` (used later by the Audio command action). */
	commands: CommandInfo[] = [];

	/** The app's version, from `welcome`. */
	version: string | undefined;

	private readonly dir: string;
	private socket: WebSocket | undefined;
	private retryTimer: NodeJS.Timeout | undefined;
	private retryUntil = 0;
	private readonly changeListeners = new Set<() => void>();
	private readonly errorListeners = new Set<(error: RemoteError) => void>();

	/**
	 * Creates the connection for one app.
	 * @param app The app's discovery name, e.g. "audio" (the `<app>.json` stem and `apps.json` key).
	 * @param exeName The app's exe file name, as monitored in the manifest.
	 */
	constructor(
		readonly app: string,
		readonly exeName: string,
	) {
		this.dir = join(process.env["APPDATA"] ?? "", "Cutmark", "remote");
	}

	/** Calls `listener` whenever the connection or the state changes; returns an unsubscribe function. */
	onChange(listener: () => void): () => void {
		this.changeListeners.add(listener);
		return () => this.changeListeners.delete(listener);
	}

	/** Calls `listener` for every `error` message from the app; returns an unsubscribe function. */
	onError(listener: (error: RemoteError) => void): () => void {
		this.errorListeners.add(listener);
		return () => this.errorListeners.delete(listener);
	}

	/** Sends a command to the app; a no-op while not running or in "Update" mode. */
	send(command: string, args?: Record<string, unknown>): void {
		if (!this.running || !this.socket || this.socket.readyState !== WebSocket.OPEN) return;
		this.socket.send(JSON.stringify(args ? { type: "command", id: command, args } : { type: "command", id: command }));
	}

	/** Spawns the app's exe from `apps.json`, detached, and watches for it to come up. */
	launch(): void {
		let exe: string | undefined;
		try {
			const apps = JSON.parse(readFileSync(join(this.dir, "apps.json"), "utf8")) as AppsFile;
			exe = apps[this.app]?.exe;
		} catch {
			exe = undefined;
		}
		if (!exe) {
			streamDeck.logger.error(`Cannot launch ${this.app}: apps.json has no exe for it`);
			return;
		}
		try {
			spawn(exe, [], { detached: true, stdio: "ignore" }).unref();
		} catch (err) {
			streamDeck.logger.error(`Cannot launch ${this.app}: ${String(err)}`);
			return;
		}
		this.beginConnecting();
	}

	/** The monitored app launched (or the plugin starts with it already up): begin the connect window. */
	appLaunched(): void {
		this.beginConnecting();
	}

	/** The monitored app quit: stop retrying, drop the socket, keys go back to "Open". */
	appTerminated(): void {
		this.stopRetrying();
		this.dropSocket();
	}

	/** Connects now and retries every 2 s for 30 s, then waits for the next launch event. */
	beginConnecting(): void {
		this.retryUntil = Date.now() + RETRY_WINDOW_MS;
		if (this.retryTimer) return;
		void this.attempt();
		this.retryTimer = setInterval(() => {
			if (Date.now() >= this.retryUntil) {
				this.stopRetrying();
				return;
			}
			void this.attempt();
		}, RETRY_MS);
	}

	private stopRetrying(): void {
		if (this.retryTimer) {
			clearInterval(this.retryTimer);
			this.retryTimer = undefined;
		}
		this.retryUntil = 0;
	}

	/** One connect attempt; skipped while a socket is already open or connecting. */
	private attempt(): void {
		if (this.socket) return;
		const discovery = readDiscovery(this.dir, this.app);
		if (!discovery || !pidRunning(discovery.pid)) return;
		const socket = new WebSocket(`ws://127.0.0.1:${discovery.port}`);
		this.socket = socket;
		socket.on("open", () => {
			if (this.socket !== socket) {
				socket.close();
				return;
			}
			socket.send(JSON.stringify({ type: "hello", token: discovery.token, client: "streamdeck", protocol: PROTOCOL }));
		});
		socket.on("message", (data) => this.receive(data, socket));
		socket.on("close", () => this.closed(socket));
		socket.on("error", () => {
			// The close event follows and the retry window keeps going.
		});
	}

	/** Handles one message from the app, per Protocol v1. */
	private receive(data: RawData, socket: WebSocket): void {
		if (this.socket !== socket) return;
		let message: Record<string, unknown>;
		try {
			message = JSON.parse(Array.isArray(data) ? Buffer.concat(data).toString() : data.toString()) as Record<string, unknown>;
		} catch {
			return;
		}
		switch (message["type"]) {
			case "welcome": {
				const protocol = message["protocol"];
				if (typeof protocol !== "number" || Math.trunc(protocol) !== PROTOCOL) {
					this.updateMode = true;
					this.running = false;
					this.state = undefined;
					socket.close();
					this.emitChange();
					return;
				}
				this.updateMode = false;
				this.running = true;
				this.version = typeof message["version"] === "string" ? message["version"] : undefined;
				this.commands = readCommands(message["commands"]);
				this.emitChange();
				return;
			}
			case "state": {
				const state = message["state"];
				this.state = state && typeof state === "object" && !Array.isArray(state) ? (state as AppState) : {};
				this.emitChange();
				return;
			}
			case "patch": {
				const patch = message["patch"];
				if (patch && typeof patch === "object" && !Array.isArray(patch)) {
					this.state = { ...(this.state ?? {}), ...(patch as AppState) };
					this.emitChange();
				}
				return;
			}
			case "error": {
				const code = message["code"];
				if (typeof code !== "string") return;
				this.emitError({
					code,
					message: typeof message["message"] === "string" ? message["message"] : code,
					id: typeof message["id"] === "string" ? message["id"] : undefined,
				});
				return;
			}
		}
	}

	/** Cleans up after the socket closed on its own; keeps the "Update" mode set by `welcome`. */
	private closed(socket: WebSocket): void {
		if (this.socket !== socket) return;
		this.socket = undefined;
		if (this.running) {
			this.running = false;
			this.state = undefined;
			this.emitChange();
		}
	}

	/** Closes the socket for good (app terminated); everything goes back to "Open". */
	private dropSocket(): void {
		const socket = this.socket;
		this.socket = undefined;
		if (socket) {
			socket.removeAllListeners();
			socket.terminate();
		}
		if (this.running || this.updateMode) {
			this.running = false;
			this.updateMode = false;
			this.state = undefined;
			this.emitChange();
		}
	}

	private emitChange(): void {
		for (const listener of [...this.changeListeners]) listener();
	}

	private emitError(error: RemoteError): void {
		for (const listener of [...this.errorListeners]) listener(error);
	}
}
