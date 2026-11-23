import streamDeck from "@elgato/streamdeck";

import { AppConnection } from "./connection";
import { AudioCommandKey } from "./actions/audio-command";
import {
	AddMarkerKey,
	JumpToEndKey,
	JumpToStartKey,
	NextMarkerKey,
	PauseKey,
	PlayStopKey,
	PreviousMarkerKey,
	RecordKey,
	RedoKey,
	RippleKey,
	SaveKey,
	SnapKey,
	SplitAtPlayheadKey,
	TrackArmKey,
	TrackMuteKey,
	TrackSoloKey,
	UndoKey,
} from "./actions/keys";
import { CommandKeyAction } from "./actions/command-key";

streamDeck.logger.setLevel("trace");

// The audio editor's connection; one AppConnection per app (spec: Remote Control "Plugin").
const connection = new AppConnection("audio", "cutmark-audio.exe");

// The key actions of the "Cutmark Audio" category (1.3-StreamDeck "Keys").
const keys: CommandKeyAction[] = [
	new PlayStopKey(),
	new PauseKey(),
	new RecordKey(),
	new JumpToStartKey(),
	new JumpToEndKey(),
	new AddMarkerKey(),
	new PreviousMarkerKey(),
	new NextMarkerKey(),
	new UndoKey(),
	new RedoKey(),
	new SplitAtPlayheadKey(),
	new RippleKey(),
	new SnapKey(),
	new SaveKey(),
	new TrackMuteKey(),
	new TrackSoloKey(),
	new TrackArmKey(),
	new AudioCommandKey(),
];
for (const key of keys) key.use(connection);

// Follow the monitored audio editor: reconnect on launch, drop the socket on quit.
streamDeck.system.onApplicationDidLaunch((ev) => {
	if (ev.application.includes("cutmark-audio")) connection.appLaunched();
});
streamDeck.system.onApplicationDidTerminate((ev) => {
	if (ev.application.includes("cutmark-audio")) connection.appTerminated();
});

for (const key of keys) streamDeck.actions.registerAction(key);

// The app may already run when the plugin starts.
connection.beginConnecting();

streamDeck.connect();
