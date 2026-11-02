<!-- Custom title bar (the window has no Windows frame). Close hides ViuMic to the tray. -->
<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import Copy from '@lucide/svelte/icons/copy'
  import Minus from '@lucide/svelte/icons/minus'
  import Square from '@lucide/svelte/icons/square'
  import X from '@lucide/svelte/icons/x'
  import { Button } from '$lib/components/ui/button/index.js'
  import logo from '../../../assets/logo.svg'

  const win = getCurrentWindow()
  let maximized = $state(false)

  $effect(() => {
    const sync = async () => (maximized = await win.isMaximized())
    sync()
    const unlisten = win.onResized(sync)
    return () => {
      unlisten.then((off) => off())
    }
  })

  const control = 'h-10 w-11 rounded-none text-muted-foreground hover:text-foreground'
</script>

<header data-tauri-drag-region class="flex h-10 shrink-0 items-center gap-2 border-b pl-3">
  <img src={logo} alt="" class="pointer-events-none size-5" />
  <span data-tauri-drag-region class="text-sm font-medium">ViuMic</span>
  <div class="ml-auto flex">
    <Button variant="ghost" size="icon" class={control} aria-label="Minimize" onclick={() => win.minimize()}>
      <Minus />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class={control}
      aria-label={maximized ? 'Restore' : 'Maximize'}
      onclick={() => win.toggleMaximize()}
    >
      {#if maximized}<Copy class="size-3.5" />{:else}<Square class="size-3.5" />{/if}
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="{control} hover:bg-destructive hover:text-white"
      aria-label="Close to tray"
      onclick={() => win.close()}
    >
      <X />
    </Button>
  </div>
</header>
