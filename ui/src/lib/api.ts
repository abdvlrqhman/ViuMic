// Typed bridge to the Rust side (src-tauri/src/lib.rs). Field names match its serde camelCase.
import { Channel, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

/** Strength at or above this removes all the noise it can. */
export const ALL_NOISE_DB = 100

export type Quality = 'best' | 'light'

export interface Settings {
  /** The chosen microphone; ViuMic always uses it and waits for it when unplugged. */
  input: string | null
  enabled: boolean
  strengthDb: number
  quality: Quality
  /** Mains hum to remove (50 or 60 Hz); null = off. */
  humHz: number | null
}

export interface View {
  settings: Settings
  /** Connected microphones, without the virtual cable. */
  mics: string[]
  error: string | null
  cableMissing: boolean
  /** The cable is a default speaker (apps, media or calls): that sound vanishes into it. */
  feedIsSpeaker: boolean
  /** "ViuMic Microphone" is Windows' default mic, so mute keys and "Default" apps use it. */
  micIsDefault: boolean
  /** What apps call ViuMic's clean mic, e.g. "ViuMic Microphone". */
  appMic: string | null
  /** Muted in Windows (mute key, taskbar): ViuMic sends silence everywhere. */
  muted: boolean
  /** ViuMic's output is hidden from screen sharing. */
  privateOutput: boolean
  /** Why it isn't, when Windows refused. */
  privateNote: string | null
}

export interface Meters {
  /** Linear peak (0..1) since the last update. */
  inPeak: number
  outPeak: number
  latencyMs: number
  /** ViuMic's share of the whole processor, in percent, as Task Manager counts it. */
  cpuPercent: number
}

export const getState = () => invoke<View>('get_state')

export const setSettings = (settings: Settings) => invoke<View>('set_settings', { settings })

/** Makes "ViuMic Microphone" Windows' default mic for every role. */
export const makeDefaultMic = () => invoke<View>('make_default_mic')

/** Moves every speaker role off the cable, back to real speakers. */
export const fixSpeaker = () => invoke<View>('fix_speaker')

/** About 30 updates a second while the window lives. */
export function watchMeters(onMeters: (m: Meters) => void) {
  const channel = new Channel<Meters>()
  channel.onmessage = onMeters
  return invoke('watch_meters', { channel })
}

/** Changes made elsewhere (tray toggle, device lost/back). */
export const onState = (f: (v: View) => void) => listen<View>('state', (e) => f(e.payload))
