<script lang="ts">
  import { onMount } from 'svelte'
  import { Tween } from 'svelte/motion'
  import { openUrl } from '@tauri-apps/plugin-opener'
  import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart'
  import Cpu from '@lucide/svelte/icons/cpu'
  import Mic from '@lucide/svelte/icons/mic'
  import Timer from '@lucide/svelte/icons/timer'
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert'
  import * as Alert from '$lib/components/ui/alert/index.js'
  import { Badge } from '$lib/components/ui/badge/index.js'
  import { Button } from '$lib/components/ui/button/index.js'
  import * as Card from '$lib/components/ui/card/index.js'
  import { Label } from '$lib/components/ui/label/index.js'
  import * as RadioGroup from '$lib/components/ui/radio-group/index.js'
  import * as Select from '$lib/components/ui/select/index.js'
  import { Slider } from '$lib/components/ui/slider/index.js'
  import { Switch } from '$lib/components/ui/switch/index.js'
  import Strip from '$lib/Strip.svelte'
  import TitleBar from '$lib/TitleBar.svelte'
  import {
    ALL_NOISE_DB,
    fixSpeaker,
    getState,
    makeDefaultMic,
    onState,
    setSettings,
    watchMeters,
    type Meters,
    type Quality,
    type Settings,
    type View,
  } from '$lib/api'

  /** Strength slider: 6..48 dB of reduction in steps of 3, then a last stop meaning "maximum". */
  const STRENGTH_MIN = 6
  const STRENGTH_MAX = 51
  const VB_CABLE_URL = 'https://vb-audio.com/Cable/'
  const HUM = [
    { value: '50', label: '50 Hz (most of the world)' },
    { value: '60', label: '60 Hz (the Americas)' },
    { value: 'off', label: 'Off' },
  ]

  let view = $state<View>()
  // Delay and CPU: median of the last ~3 s (90 updates), refreshed twice a second and
  // eased, so the numbers read steady instead of flickering every 33 ms.
  const STATS_WINDOW = 90
  const latencyHistory: number[] = []
  const cpuHistory: number[] = []
  const delayMs = new Tween(0, { duration: 400 })
  const cpuPercent = new Tween(0, { duration: 400 })
  const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[xs.length >> 1] ?? 0
  const cpuText = (p: number) => (p < 10 ? `${p.toFixed(1)}%` : `${Math.round(p)}%`)
  const remember = (xs: number[], x: number) => {
    xs.push(x)
    if (xs.length > STATS_WINDOW) xs.shift()
  }
  let meters = $state<Meters>({ inPeak: 0, outPeak: 0, latencyMs: 0, cpuPercent: 0 })
  let startWithWindows = $state(false)
  // Dev builds don't register autostart (see src-tauri): the switch says so instead.
  let autostartAvailable = $state(true)
  // Follows the saved setting, but moves freely while dragging.
  let strength = $state(STRENGTH_MAX)

  $effect(() => {
    if (view)
      strength = view.settings.strengthDb >= ALL_NOISE_DB ? STRENGTH_MAX : view.settings.strengthDb
  })

  onMount(() => {
    const refresh = () => getState().then((v) => (view = v))
    refresh()
    watchMeters((m) => {
      meters = m
      remember(latencyHistory, m.latencyMs)
      remember(cpuHistory, m.cpuPercent)
    })
    const stats = setInterval(() => {
      delayMs.target = median(latencyHistory)
      cpuPercent.target = median(cpuHistory)
    }, 500)
    isEnabled()
      .then((on) => (startWithWindows = on))
      .catch(() => (autostartAvailable = false))
    const unlisten = onState((v) => (view = v))
    // Mics come and go (USB, Bluetooth): refresh the list whenever the window comes back.
    window.addEventListener('focus', refresh)
    return () => {
      unlisten.then((off) => off())
      window.removeEventListener('focus', refresh)
      clearInterval(stats)
    }
  })

  async function update(patch: Partial<Settings>) {
    if (view) view = await setSettings({ ...view.settings, ...patch })
  }

  async function setStartWithWindows(on: boolean) {
    await (on ? enable() : disable())
    startWithWindows = await isEnabled()
  }

  const on = $derived(view?.settings.enabled ?? true)
  const mic = $derived(view?.settings.input ?? null)
  const micConnected = $derived(!!mic && !!view?.mics.includes(mic))
  const micOptions = $derived(view ? (mic && !micConnected ? [mic, ...view.mics] : view.mics) : [])
  const appMic = $derived(view?.appMic ?? 'ViuMic Microphone')
  const muted = $derived(view?.muted ?? false)
  const status = $derived(
    view?.error ? 'Needs attention' : muted ? 'Muted' : on ? 'Cleaning' : 'Off',
  )
  let fixError = $state<string | null>(null)
  async function fix(action: () => Promise<View>) {
    try {
      view = await action()
      fixError = null
    } catch (e) {
      fixError = String(e)
    }
  }
</script>

<div class="flex h-screen flex-col overflow-hidden bg-background text-foreground">
  <TitleBar />

  <main class="flex-1 overflow-y-auto">
    <div class="mx-auto grid max-w-3xl gap-4 p-4 sm:grid-cols-2 sm:p-6">
      {#if view?.cableMissing}
        <Alert.Root variant="destructive" class="sm:col-span-2">
          <TriangleAlert />
          <Alert.Title>The ViuMic microphone isn't installed</Alert.Title>
          <Alert.Description>
            Reinstall ViuMic to add it, or install VB-CABLE yourself and restart your PC.
            <Button variant="link" class="h-auto p-0" onclick={() => openUrl(VB_CABLE_URL)}>
              Get VB-CABLE
            </Button>
          </Alert.Description>
        </Alert.Root>
      {:else}
        {#if view?.feedIsSpeaker}
          <Alert.Root class="sm:col-span-2">
            <TriangleAlert />
            <Alert.Title>Some sound is going into ViuMic instead of your speakers</Alert.Title>
            <Alert.Description>
              Windows plays calls or other sound into ViuMic Feed, so you can't hear it.
              <Button variant="link" class="h-auto p-0" onclick={() => fix(fixSpeaker)}>
                Send it back to my speakers
              </Button>
            </Alert.Description>
          </Alert.Root>
        {/if}
        {#if view && !view.micIsDefault}
          <Alert.Root class="sm:col-span-2">
            <Mic />
            <Alert.Title>Make ViuMic your default microphone</Alert.Title>
            <Alert.Description>
              Then your laptop's mute key mutes ViuMic, and apps set to "Default" use your clean
              voice.
              <Button variant="link" class="h-auto p-0" onclick={() => fix(makeDefaultMic)}>
                Make ViuMic the default
              </Button>
            </Alert.Description>
          </Alert.Root>
        {/if}
        {#if view && !view.privateOutput && view.privateNote}
          <Alert.Root class="sm:col-span-2">
            <TriangleAlert />
            <Alert.Title>Screen shares might include your voice</Alert.Title>
            <Alert.Description>
              ViuMic couldn't hide its audio from screen sharing: {view.privateNote}.
            </Alert.Description>
          </Alert.Root>
        {/if}
        {#if fixError}
          <Alert.Root variant="destructive" class="sm:col-span-2">
            <TriangleAlert />
            <Alert.Title>That didn't work</Alert.Title>
            <Alert.Description>{fixError}</Alert.Description>
          </Alert.Root>
        {/if}
      {/if}
      {#if view?.error && !view.cableMissing}
        <Alert.Root variant="destructive" class="sm:col-span-2">
          <TriangleAlert />
          <Alert.Title>Your voice isn't reaching apps</Alert.Title>
          <Alert.Description>{view.error}.</Alert.Description>
        </Alert.Root>
      {/if}

      <Card.Root class="sm:col-span-2">
        <Card.Header>
          <Card.Title class="flex items-center gap-2">
            Noise removal
            <Badge variant={view?.error ? 'destructive' : muted || !on ? 'secondary' : 'default'}>{status}</Badge>
          </Card.Title>
          <Card.Description>
            {muted
              ? 'Muted in Windows. Apps and screen shares hear silence.'
              : on
                ? 'Apps hear your voice without the background.'
                : 'Apps hear your mic as it is.'}
          </Card.Description>
          <Card.Action>
            <Switch
              checked={on}
              disabled={!view}
              aria-label="Noise removal"
              class="data-checked:bg-chart-2"
              onCheckedChange={(checked) => update({ enabled: checked })}
            />
          </Card.Action>
        </Card.Header>
        <Card.Content class="space-y-2">
          <Strip {meters} enabled={on} />
          <div class="flex items-center gap-4 text-xs text-muted-foreground">
            <span class="flex items-center gap-1.5">
              <span class="size-2 rounded-full bg-muted-foreground/50"></span>Your mic
            </span>
            <span class="flex items-center gap-1.5">
              <span class="size-2 rounded-full bg-chart-2"></span>What apps hear
            </span>
          </div>
        </Card.Content>
        <Card.Footer class="gap-4 text-xs text-muted-foreground tabular-nums">
          <span class="flex items-center gap-1.5"><Timer class="size-3.5" />{Math.round(delayMs.current)} ms delay</span>
          <span class="flex items-center gap-1.5" title="ViuMic's share of your whole processor, like Task Manager shows it">
            <Cpu class="size-3.5" />{cpuText(cpuPercent.current)} CPU
          </span>
        </Card.Footer>
      </Card.Root>

      <Card.Root>
        <Card.Header>
          <Card.Title>Microphone</Card.Title>
          <Card.Description>ViuMic always uses this mic, and waits for it if it's unplugged.</Card.Description>
        </Card.Header>
        <Card.Content>
          <Select.Root type="single" value={mic ?? undefined} onValueChange={(v) => update({ input: v })}>
            <Select.Trigger class="w-full" aria-label="Microphone">
              <span class="flex min-w-0 flex-1 items-center gap-2 text-left">
                <Mic class="shrink-0 text-muted-foreground" />
                <span class="truncate">
                  {mic ?? 'Choose a microphone'}{mic && !micConnected ? ' (not connected)' : ''}
                </span>
              </span>
            </Select.Trigger>
            <Select.Content>
              {#each micOptions as name (name)}
                <Select.Item value={name} label={name}>
                  {name}{name === mic && !micConnected ? ' (not connected)' : ''}
                </Select.Item>
              {/each}
            </Select.Content>
          </Select.Root>
        </Card.Content>
      </Card.Root>

      <Card.Root>
        <Card.Header>
          <Card.Title>Strength</Card.Title>
          <Card.Description>How much background noise to remove.</Card.Description>
          <Card.Action class="text-sm tabular-nums text-muted-foreground">
            {strength >= STRENGTH_MAX ? 'Maximum' : `−${strength} dB`}
          </Card.Action>
        </Card.Header>
        <Card.Content>
          <Slider
            type="single"
            min={STRENGTH_MIN}
            max={STRENGTH_MAX}
            step={3}
            disabled={!on}
            aria-label="Strength"
            bind:value={strength}
            onValueCommit={(v) => update({ strengthDb: v >= STRENGTH_MAX ? ALL_NOISE_DB : v })}
          />
        </Card.Content>
      </Card.Root>

      <Card.Root>
        <Card.Header>
          <Card.Title>Use it in your apps</Card.Title>
          <Card.Description>
            In Discord, Zoom, OBS or any app, choose <span class="font-medium text-foreground">{appMic}</span> as
            your microphone, and turn off the app's own noise suppression.
            {#if view?.privateOutput}
              Your voice is hidden from screen sharing, so muting in Discord mutes you everywhere.
            {/if}
          </Card.Description>
        </Card.Header>
        <Card.Footer class="text-xs text-muted-foreground">
          <p>
            The ViuMic microphone is VB-CABLE by VB-Audio, a donationware driver.
            <Button variant="link" class="h-auto p-0 text-xs" onclick={() => openUrl(VB_CABLE_URL)}>
              Support VB-Audio
            </Button>
          </p>
        </Card.Footer>
      </Card.Root>

      <Card.Root>
        <Card.Header>
          <Card.Title>Preferences</Card.Title>
        </Card.Header>
        <Card.Content class="space-y-5">
          <div class="space-y-2">
            <Label>Quality</Label>
            <RadioGroup.Root
              value={view?.settings.quality ?? 'best'}
              onValueChange={(v) => update({ quality: v as Quality })}
              class="flex gap-6"
            >
              <div class="flex items-center gap-2">
                <RadioGroup.Item value="best" id="quality-best" />
                <Label for="quality-best" class="font-normal">Best</Label>
              </div>
              <div class="flex items-center gap-2">
                <RadioGroup.Item value="light" id="quality-light" />
                <Label for="quality-light" class="font-normal">Light, for older PCs</Label>
              </div>
            </RadioGroup.Root>
          </div>
          <div class="space-y-2">
            <Label>Remove electrical hum</Label>
            <Select.Root
              type="single"
              value={String(view?.settings.humHz ?? 'off')}
              onValueChange={(v) => update({ humHz: v === 'off' ? null : Number(v) })}
            >
              <Select.Trigger class="w-full" aria-label="Remove electrical hum">
                <span class="flex-1 truncate text-left">
                  {HUM.find((h) => h.value === String(view?.settings.humHz ?? 'off'))?.label}
                </span>
              </Select.Trigger>
              <Select.Content>
                {#each HUM as h (h.value)}
                  <Select.Item value={h.value} label={h.label}>{h.label}</Select.Item>
                {/each}
              </Select.Content>
            </Select.Root>
          </div>
          <div class="flex items-center justify-between gap-4">
            <Label for="autostart" class="flex-col items-start gap-1">
              Start with Windows
              <span class="font-normal text-muted-foreground">
                {autostartAvailable ? 'Opens quietly in the tray.' : 'Only in the installed app.'}
              </span>
            </Label>
            <Switch
              id="autostart"
              checked={startWithWindows}
              disabled={!autostartAvailable}
              onCheckedChange={setStartWithWindows}
            />
          </div>
        </Card.Content>
      </Card.Root>
    </div>
  </main>
</div>
