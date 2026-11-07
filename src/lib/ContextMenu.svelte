<script>
  import Icon from "./Icon.svelte";
  import { ui } from "./ui.svelte.js";

  let el = $state();
  let pos = $state({ x: 0, y: 0 });
  let sub = $state(null); // index du sous-menu ouvert

  // Garde le menu dans la fenêtre
  $effect(() => {
    if (!ui.menu || !el) return;
    const { innerWidth: W, innerHeight: H } = window;
    const r = el.getBoundingClientRect();
    pos = { x: Math.min(ui.menu.x, W - r.width - 8), y: Math.min(ui.menu.y, H - r.height - 8) };
  });

  $effect(() => {
    if (!ui.menu) return;
    sub = null;
    const close = () => (ui.menu = null);
    const key = (e) => e.key === "Escape" && close();
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", key);
    document.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", key);
      document.removeEventListener("scroll", close, true);
    };
  });

  function run(item) {
    if (item.disabled || item.sub) return;
    ui.menu = null;
    item.run?.();
  }
</script>

{#if ui.menu}
  <div
    class="menu"
    bind:this={el}
    style="left:{pos.x}px;top:{pos.y}px"
    role="menu"
    tabindex="-1"
    onpointerdown={(e) => e.stopPropagation()}
    oncontextmenu={(e) => e.preventDefault()}
  >
    {#each ui.menu.items.filter(Boolean) as item, i}
      {#if item === "-"}
        <div class="sep"></div>
      {:else}
        <div class="row" role="none" onpointerenter={() => (sub = item.sub ? i : null)}>
          <button class="item" class:danger={item.danger} disabled={item.disabled} role="menuitem" onclick={() => run(item)}>
            <span class="ic">{#if item.icon}<Icon name={item.icon} size={15} />{/if}</span>
            <span class="label">{item.label}</span>
            {#if item.checked}<Icon name="check" size={14} />{/if}
            {#if item.sub}<Icon name="chevRight" size={14} />{/if}
          </button>
          {#if item.sub && sub === i}
            <div class="menu submenu">
              {#each item.sub as s}
                <button class="item" role="menuitem" onclick={() => run(s)}>
                  <span class="ic">{#if s.checked}<Icon name="check" size={14} />{/if}</span>
                  <span class="label">{s.label}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    z-index: 200;
    min-width: 230px;
    padding: 5px;
    border-radius: 11px;
    background: var(--menu-bg);
    border: 1px solid var(--line-strong);
    box-shadow: var(--shadow-lg);
    backdrop-filter: blur(20px);
    animation: pop 0.09s ease-out;
  }
  @keyframes pop { from { opacity: 0; transform: scale(0.97); } }
  .row { position: relative; }
  .item {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 7px 10px;
    border: 0;
    border-radius: 7px;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .item:hover:not(:disabled) { background: var(--accent); color: white; }
  .item:disabled { opacity: 0.4; cursor: default; }
  .item.danger { color: var(--err); }
  .item.danger:hover { background: var(--err); color: white; }
  .ic { width: 15px; display: grid; place-items: center; flex: none; }
  .label { flex: 1; white-space: nowrap; }
  .sep { height: 1px; margin: 5px 8px; background: var(--line); }
  .submenu { position: absolute; left: calc(100% + 2px); top: -5px; animation: none; max-height: 320px; overflow-y: auto; }
</style>
