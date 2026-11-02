<!--
  Live strip: the raw mic level (lavender) behind the cleaned voice (mint), newest on the right.
  Whatever lavender shows around the mint is the noise ViuMic is removing.
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

    const style = getComputedStyle(canvas)
    const mid = h / 2
    // Mirrored envelope around the centre line, like a waveform.
    const band = (data: Float32Array, color: string) => {
      ctx.beginPath()
      for (let i = 0; i < POINTS; i++) ctx.lineTo((i / (POINTS - 1)) * w, mid - data[i] * mid * 0.92)
      for (let i = POINTS - 1; i >= 0; i--) ctx.lineTo((i / (POINTS - 1)) * w, mid + data[i] * mid * 0.92)
      ctx.closePath()
      ctx.fillStyle = color
      ctx.fill()
    }
    band(raw, style.getPropertyValue('--noise'))
    band(clean, style.getPropertyValue(on ? '--clean' : '--text-dim'))
    ctx.fillStyle = style.getPropertyValue('--line')
    ctx.fillRect(0, mid - 0.5, w, 1)
  }
</script>

<div
  class="strip"
  role="img"
  aria-label="Live levels: your microphone in lavender, the cleaned voice in mint"
  bind:clientWidth={width}
  bind:clientHeight={height}
>
  <canvas bind:this={canvas}></canvas>
</div>

<style>
  .strip {
    height: 104px;
    border-radius: 14px;
    background: var(--surface);
    overflow: hidden;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
