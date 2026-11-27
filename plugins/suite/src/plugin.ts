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
import {
	RecorderHighlightKey,
	RecorderMarkKey,
	RecorderMarkLabelKey,
	RecorderMuteMicKey,
	RecorderPauseKey,
	RecorderRecordKey,
	RecorderReplayKey,
	RecorderSaveReplayKey,
	RecorderTrackMuteKey,
} from "./actions/recorder-keys";
import { JogDial, MarkersDial, TrackVolumeDial, ZoomDial } from "./actions/dials";
import { DialBase } from "./actions/dial";

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

// The dial actions of the "Cutmark Audio" category (1.3-StreamDeck "Dials").
const dials: DialBase[] = [new JogDial(), new ZoomDial(), new TrackVolumeDial(), new MarkersDial()];
for (const dial of dials) dial.use(connection);

// The Recorder's connection and its keys (spec: recorder-stream-deck "Plugin: Recorder keys").
const recorder = new AppConnection("recorder", "cutmark-recorder.exe");
const recorderKeys: CommandKeyAction[] = [
	new RecorderRecordKey(),
	new RecorderPauseKey(),
	new RecorderMarkKey(),
	new RecorderMarkLabelKey(),
	new RecorderHighlightKey(),
	new RecorderMuteMicKey(),
	new RecorderReplayKey(),
	new RecorderSaveReplayKey(),
	new RecorderTrackMuteKey(),
];
for (const key of recorderKeys) key.use(recorder);

// Follow the monitored apps: reconnect on launch, drop the socket on quit.
streamDeck.system.onApplicationDidLaunch((ev) => {
	if (ev.application.includes("cutmark-audio")) connection.appLaunched();
	if (ev.application.includes("cutmark-recorder")) recorder.appLaunched();
});
streamDeck.system.onApplicationDidTerminate((ev) => {
	if (ev.application.includes("cutmark-audio")) connection.appTerminated();
	if (ev.application.includes("cutmark-recorder")) recorder.appTerminated();
});

for (const action of [...keys, ...dials, ...recorderKeys]) streamDeck.actions.registerAction(action);

// The apps may already run when the plugin starts.
connection.beginConnecting();
recorder.beginConnecting();

streamDeck.connect();
