<!--
  Live chart: your raw mic level (muted) behind the cleaned voice (emerald), newest on the right.
  The muted area around the emerald one is the noise ViuMic is removing.
-->
<script lang="ts">
  import { untrack } from 'svelte'
  import type { Meters } from './api'

  let { meters, enabled }: { meters: Meters; enabled: boolean } = $props()

  /** 6 s of history at ~30 updates a second. */
  const POINTS = 180
  /** dBFS shown at the bottom of the scale. */
  const FLOOR_DB = -66

  const raw = new Float32Array(POINTS)
  const clean = new Float32Array(POINTS)
  let canvas: HTMLCanvasElement
  let width = $state(0)
  let height = $state(0)

  const level = (peak: number) =>
    Math.max(0, 1 - (20 * Math.log10(Math.max(peak, 1e-6))) / FLOOR_DB)

  // New meters: scroll one point in. Only `meters` is tracked here.
  $effect(() => {
    raw.copyWithin(0, 1)
    clean.copyWithin(0, 1)
    raw[POINTS - 1] = level(meters.inPeak)
    clean[POINTS - 1] = level(meters.outPeak)
    untrack(draw)
  })

  // Resize or on/off: redraw what we have.
  $effect(draw)

  function draw() {
    const [w, h, on] = [width, height, enabled]
    const ctx = canvas?.getContext('2d')
    if (!ctx || !w || !h) return
    const dpr = devicePixelRatio
    if (canvas.width !== Math.round(w * dpr)) canvas.width = Math.round(w * dpr)
    if (canvas.height !== Math.round(h * dpr)) canvas.height = Math.round(h * dpr)
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    ctx.clearRect(0, 0, w, h)

    const css = getComputedStyle(canvas)
    const mid = h / 2
    // Mirrored envelope around the centre line, like a waveform.
    const band = (data: Float32Array, color: string, alpha: number) => {
      ctx.beginPath()
      for (let i = 0; i < POINTS; i++) ctx.lineTo((i / (POINTS - 1)) * w, mid - data[i] * mid * 0.9)
      for (let i = POINTS - 1; i >= 0; i--) ctx.lineTo((i / (POINTS - 1)) * w, mid + data[i] * mid * 0.9)
      ctx.closePath()
      ctx.globalAlpha = alpha
      ctx.fillStyle = color
      ctx.fill()
    }
    band(raw, css.getPropertyValue('--muted-foreground'), 0.35)
    band(clean, css.getPropertyValue(on ? '--chart-2' : '--muted-foreground'), on ? 1 : 0.7)
    ctx.globalAlpha = 1
  }
</script>

<div
  class="h-28 w-full overflow-hidden rounded-md border bg-muted/30"
  role="img"
  aria-label="Live levels: your microphone, and the cleaned voice in green"
  bind:clientWidth={width}
  bind:clientHeight={height}
>
  <canvas bind:this={canvas} class="block size-full"></canvas>
</div>
