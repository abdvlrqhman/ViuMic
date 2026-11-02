<script lang="ts">
  import { onMount } from 'svelte'
  import { openUrl } from '@tauri-apps/plugin-opener'
  import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart'
  import logo from '../../assets/logo.svg'
  import Strip from './lib/Strip.svelte'
  import {
    ALL_NOISE_DB,
    getState,
    onState,
    setSettings,
    watchMeters,
    type Meters,
    type Settings,
    type View,
  } from './lib/api'

  /** Strength slider: 6..48 dB of reduction in steps of 3, then one last stop meaning "all". */
  const STRENGTH_MIN = 6
  const STRENGTH_ALL = 51
  const DEPTH_MAX = 6
  const VB_CABLE_URL = 'https://vb-audio.com/Cable/'

  let view = $state<View>()
  let meters = $state<Meters>({ inPeak: 0, outPeak: 0, latencyMs: 0, load: 0 })
  let startWithWindows = $state(false)
  // Slider positions follow the saved settings, but move freely while dragging.
  let strength = $state(STRENGTH_ALL)
  let depth = $state(0)

  $effect(() => {
    if (!view) return
    strength = view.settings.strengthDb >= ALL_NOISE_DB ? STRENGTH_ALL : view.settings.strengthDb
    depth = view.settings.depth
  })

  onMount(() => {
    const refresh = () => getState().then((v) => (view = v))
    refresh()
    watchMeters((m) => (meters = m))
    isEnabled()
      .then((on) => (startWithWindows = on))
      .catch(() => {})
    const unlisten = onState((v) => (view = v))
    // Mics come and go (USB, Bluetooth): refresh the list whenever the window comes back.
    window.addEventListener('focus', refresh)
    return () => {
      unlisten.then((off) => off())
      window.removeEventListener('focus', refresh)
    }
  })

  async function update(patch: Partial<Settings>) {
    if (view) view = await setSettings({ ...view.settings, ...patch })
  }

  async function toggleStartWithWindows() {
    await (startWithWindows ? disable() : enable())
    startWithWindows = await isEnabled()
  }

  const fill = (v: number, min: number, max: number) => `${((v - min) / (max - min)) * 100}%`
  const strengthText = $derived(strength >= STRENGTH_ALL ? 'All of it' : `${strength} dB less`)
  const depthText = $derived(depth === 0 ? 'Off' : `${depth} semitones lower`)
  const on = $derived(view?.settings.enabled ?? true)
</script>

<main>
  <header>
    <img src={logo} alt="" width="30" height="30" />
    <h1>ViuMic</h1>
    <button
      class="switch"
      role="switch"
      aria-checked={on}
      disabled={!view}
      onclick={() => update({ enabled: !on })}
    >
      Noise removal
      <span class="track" aria-hidden="true"><span class="knob"></span></span>
    </button>
  </header>

  <Strip {meters} enabled={on} />
  <p class="hint">
    In Discord, Zoom or OBS, choose <strong>CABLE Output</strong> as your microphone.
  </p>

  {#if view}
    {#if view.cableMissing}
      <div class="note" role="status">
        <p>
          ViuMic sends your clean voice to VB-CABLE, a free virtual microphone. Install it, then
          restart your PC.
        </p>
        <button onclick={() => openUrl(VB_CABLE_URL)}>Get VB-CABLE</button>
      </div>
    {:else if view.cableIsSpeaker}
      <div class="note" role="status">
        <p>
          Windows is playing all sound into VB-CABLE, so you can't hear anything. Choose your
          speakers or headphones as the output.
        </p>
        <button onclick={() => openUrl('ms-settings:sound')}>Open sound settings</button>
      </div>
    {/if}
    {#if view.error && !view.cableMissing}
      <div class="note" role="alert">
        <p>{view.error}. ViuMic tries again every 2 seconds.</p>
      </div>
    {/if}

    <label class="field">
      <span>Microphone</span>
      <select
        value={view.settings.input ?? ''}
        onchange={(e) => update({ input: e.currentTarget.value || null })}
      >
        <option value="">
          Automatic{view.mic && !view.settings.input ? ` (${view.mic})` : ''}
        </option>
        {#if view.settings.input && !view.mics.includes(view.settings.input)}
          <option value={view.settings.input}>{view.settings.input} (not connected)</option>
        {/if}
        {#each view.mics as mic (mic)}
          <option value={mic}>{mic}</option>
        {/each}
      </select>
    </label>

    <label class="field">
      <span class="row"><span>Noise to remove</span><output>{strengthText}</output></span>
      <input
        type="range"
        min={STRENGTH_MIN}
        max={STRENGTH_ALL}
        step="3"
        disabled={!on}
        bind:value={strength}
        style:--fill={fill(strength, STRENGTH_MIN, STRENGTH_ALL)}
        onchange={() => update({ strengthDb: strength >= STRENGTH_ALL ? ALL_NOISE_DB : strength })}
      />
    </label>

    <label class="field">
      <span class="row"><span>Deeper voice</span><output>{depthText}</output></span>
      <input
        type="range"
        min="0"
        max={DEPTH_MAX}
        step="0.5"
        bind:value={depth}
        style:--fill={fill(depth, 0, DEPTH_MAX)}
        onchange={() => update({ depth })}
      />
    </label>

    <details>
      <summary>More settings</summary>
      <fieldset>
        <legend>Quality</legend>
        <label class="choice">
          <input
            type="radio"
            name="quality"
            checked={view.settings.quality === 'best'}
            onchange={() => update({ quality: 'best' })}
          />
          Best
        </label>
        <label class="choice">
          <input
            type="radio"
            name="quality"
            checked={view.settings.quality === 'light'}
            onchange={() => update({ quality: 'light' })}
          />
          Light, for older PCs
        </label>
      </fieldset>
      <label class="field">
        <span>Remove electrical hum</span>
        <select
          value={String(view.settings.humHz ?? 'off')}
          onchange={(e) => {
            const v = e.currentTarget.value
            update({ humHz: v === 'off' ? null : Number(v) })
          }}
        >
          <option value="50">50 Hz (most of the world)</option>
          <option value="60">60 Hz (the Americas)</option>
          <option value="off">Off</option>
        </select>
      </label>
      <label class="choice">
        <input type="checkbox" checked={startWithWindows} onchange={toggleStartWithWindows} />
        Start with Windows
      </label>
    </details>
  {/if}

  <footer>
    <span>{Math.round(meters.latencyMs)} ms delay</span>
    <span title="Worst-case time to clean each 10 ms of audio, on one core">
      {Math.round(meters.load * 100)}% CPU
    </span>
  </footer>
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    gap: 18px;
    max-width: 520px;
    min-height: 100vh;
    margin: 0 auto;
    padding: 18px 20px 14px;
  }

  header {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  h1 {
    margin: 0;
    font-size: var(--step-2);
    font-weight: 650;
    font-stretch: 125%;
    letter-spacing: 0.01em;
  }

  .switch {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 4px 6px 10px;
    border: 0;
    border-radius: 999px;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
  }
  .switch[aria-checked='true'] {
    color: var(--text);
  }
  .track {
    width: 42px;
    height: 24px;
    padding: 3px;
    border-radius: 999px;
    background: var(--line);
    transition: background 150ms;
  }
  .knob {
    display: block;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--text-dim);
    transition:
      transform 150ms,
      background 150ms;
  }
  .switch[aria-checked='true'] .track {
    background: var(--clean-deep);
  }
  .switch[aria-checked='true'] .knob {
    transform: translateX(18px);
    background: var(--text);
  }

  .hint {
    margin: -8px 0 0;
    font-size: var(--step--1);
    color: var(--text-dim);
  }
  .hint strong {
    font-weight: 600;
    color: var(--text);
  }

  .note {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 12px 14px;
    border: 1px solid rgb(255 197 107 / 0.35);
    border-radius: 10px;
    background: rgb(255 197 107 / 0.07);
  }
  .note p {
    margin: 0;
    max-width: 60ch;
  }
  .note button {
    padding: 0;
    border: 0;
    background: none;
    color: var(--warn);
    font-weight: 600;
    text-decoration: underline;
    text-underline-offset: 3px;
    cursor: pointer;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    justify-content: space-between;
  }
  output {
    color: var(--text-dim);
  }

  select {
    appearance: none;
    width: 100%;
    padding: 9px 36px 9px 12px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: var(--surface)
      url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='8'%3E%3Cpath d='M1 1l5 5 5-5' fill='none' stroke='%239aa1d4' stroke-width='1.6'/%3E%3C/svg%3E")
      no-repeat right 14px center;
    text-overflow: ellipsis;
    cursor: pointer;
  }
  option {
    background: var(--surface);
  }

  input[type='range'] {
    appearance: none;
    width: 100%;
    height: 20px;
    margin: 0;
    background: none;
    cursor: pointer;
  }
  input[type='range']::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--clean) var(--fill), var(--line) var(--fill));
  }
  input[type='range']::-webkit-slider-thumb {
    appearance: none;
    width: 16px;
    height: 16px;
    margin-top: -6px;
    border-radius: 50%;
    background: var(--text);
  }
  input[type='range']:disabled {
    opacity: 0.4;
    cursor: default;
  }

  details {
    border-top: 1px solid var(--line);
    padding-top: 12px;
  }
  summary {
    color: var(--text-dim);
    cursor: pointer;
  }
  details[open] summary {
    margin-bottom: 14px;
  }
  details[open] {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  fieldset {
    display: flex;
    gap: 18px;
    margin: 0;
    padding: 0;
    border: 0;
  }
  legend {
    float: left;
    width: 100%;
    margin-bottom: 8px;
  }
  .choice {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  input[type='radio'],
  input[type='checkbox'] {
    accent-color: var(--clean-deep);
    margin: 0;
  }

  footer {
    display: flex;
    gap: 18px;
    margin-top: auto;
    font-size: var(--step--1);
    color: var(--text-dim);
  }
</style>
