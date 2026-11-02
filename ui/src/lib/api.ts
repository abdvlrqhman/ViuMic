// Typed bridge to the Rust side (src-tauri/src/lib.rs). Field names match its serde camelCase.
import { Channel, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

/** Strength at or above this removes all the noise it can. */
export const ALL_NOISE_DB = 100

export type Quality = 'best' | 'light'

export interface Settings {
  /** Microphone name; null = automatic (Windows default, skipping VB-CABLE). */
  input: string | null
  enabled: boolean
  strengthDb: number
  /** Deeper voice, in semitones; 0 = off. */
  depth: number
  quality: Quality
  /** Mains hum to remove (50 or 60 Hz); null = off. */
  humHz: number | null
}

export interface View {
  settings: Settings
  mics: string[]
  /** The mic in use right now. */
  mic: string | null
  error: string | null
  cableMissing: boolean
  cableIsSpeaker: boolean
}

export interface Meters {
  /** Linear peak (0..1) since the last update. */
  inPeak: number
  outPeak: number
  latencyMs: number
  /** Worst DSP time per 10 ms step, as a fraction of the budget. */
  load: number
}

export const getState = () => invoke<View>('get_state')

export const setSettings = (settings: Settings) => invoke<View>('set_settings', { settings })

/** About 30 updates a second while the window lives. */
export function watchMeters(onMeters: (m: Meters) => void) {
  const channel = new Channel<Meters>()
  channel.onmessage = onMeters
  return invoke('watch_meters', { channel })
}

/** Changes made elsewhere (tray toggle, device lost/back). */
export const onState = (f: (v: View) => void) => listen<View>('state', (e) => f(e.payload))
