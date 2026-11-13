<script>
  import Icon from "./Icon.svelte";
  import ConsoleBadge from "./ConsoleBadge.svelte";
  import { KIND_LABEL, PLAYABLE, STATUS_LABEL, fmtAgo, fmtEta, fmtSize, fmtSpeed, hueOf, isDone, isPaused, partialLabel, subtitle } from "./model.js";
  import { arts, wantArt, schedules, wantSchedule, airTime, alignEpisodes } from "./ui.svelte.js";

  let { group: g, sort, selected = false, onclick, ondblclick, onaction, oncontext } = $props();

  const hue = $derived(hueOf(g.artKey));
  const pct = $derived(Math.floor(g.progress * 1000) / 10);
  // Pause / reprise n'ont de sens que pour ce qui n'est pas fini de télécharger
  const incomplete = $derived(g.items.filter((i) => i.progress < 1 && i.status !== "error"));
  const paused = $derived(incomplete.length > 0 && incomplete.every((i) => isPaused(i.status)));
  const running = $derived(incomplete.some((i) => !isPaused(i.status)));
  const installed = $derived(g.items.find((i) => i.installed)?.installed);
  const format = $derived(g.items.find((i) => i.format)?.format);

  // Séries suivies récemment : prochain épisode (pastille « É9 · 19 h »)
  const isShow = $derived(g.kind === "series" || g.kind === "anime");
  const recent = $derived(Date.now() / 1000 - g.added_on < 120 * 86400);
  $effect(() => {
    if (isShow && recent) wantSchedule(g);
  });
  const nextAir = $derived.by(() => {
    const s = isShow ? schedules[g.artKey] : null;
    if (!s) return null;
    const now = Date.now();
    const season = Math.max(0, ...g.items.map((i) => i.season ?? 0));
    const owned = new Set(g.items.filter((i) => (i.season ?? season) === season).map((i) => i.episode).filter((n) => n != null));
    const eps = alignEpisodes(s, owned, season, now, g.items.filter((i) => (i.season ?? season) === season));
    const ep = eps.filter((e) => airTime(e) > now && !owned.has(e.number)).sort((a, b) => airTime(a) - airTime(b))[0];
    if (!ep) return null;
    const d = airTime(ep) - now;
    if (d > 8 * 86400000) return null;
    const h = Math.round(d / 3600000);
    return { n: ep.number, when: d < 3600000 ? `${Math.max(1, Math.round(d / 60000))} min` : h < 24 ? `${h} h` : `${Math.round(d / 86400000)} j` };
  });
  const when = $derived(sort === "completed" ? fmtAgo(g.completion_on) : fmtAgo(g.added_on));
  // Lecture directe : seulement un film (ou un audio) seul. Pour une série, on choisit l'épisode.

  // Affiche : demandée une seule fois par titre, avec image de secours
  const art = $derived(arts[g.artKey]);
  let src = $state(null);
  let loaded = $state(false);
  $effect(() => wantArt(g));
  $effect(() => {
    src = art?.url ?? null;
    loaded = false;
  });
  function onerror() {
    if (art?.fallback && src !== art.fallback) src = art.fallback;
    else src = null;
  }

  const RING = 2 * Math.PI * 27; // circonférence de l'anneau de progression

  function act(e, fn) {
    e.stopPropagation();
    fn();
  }
</script>

<div
  class="card s-{g.status}"
  class:stack={g.items.length > 1}
  class:paused
  class:complete={g.progress >= 1}
  class:downloading={running}
  class:selected
  style="--h:{hue}"
  role="button"
  tabindex="0"
  data-key={g.key}
  {onclick}
  {ondblclick}
  oncontextmenu={oncontext}
  onkeydown={(e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onclick(e))}
>
  <div class="cover">
    <div class="art">
      <span class="watermark"><Icon name={g.kind} size={140} /></span>
      <span class="cover-title">{g.title}</span>
    </div>
    {#if src}
      <img {src} alt="" class:loaded loading="lazy" decoding="async" draggable="false" onload={() => (loaded = true)} {onerror} />
    {/if}
    <div class="shade"></div>

    <!-- Centre de l'affiche : anneau de progression, pause, ou lecture.
         Au survol, l'anneau devient un bouton pause et la pause devient un bouton lecture. -->
    <div class="center">
      {#if running}
        <div class="slot">
          <div class="ring idle">
            <svg viewBox="0 0 64 64" aria-hidden="true">
              <circle class="track" cx="32" cy="32" r="27" />
              <circle class="fill" cx="32" cy="32" r="27" stroke-dasharray={RING} stroke-dashoffset={RING * (1 - g.progress)} />
            </svg>
            <span class="pct">{Math.floor(g.progress * 100)}<small>%</small></span>
          </div>
          <button class="cbtn hover" title="Mettre en pause" onclick={(e) => act(e, () => onaction("pause", incomplete))}><Icon name="pause" size={20} /></button>
        </div>
        <span class="eta">{g.status === "stalled" ? "en attente…" : fmtEta(g.eta) || (g.status === "checking" ? "vérification" : "")}</span>
      {:else if paused}
        <div class="slot">
          <div class="pdisk idle"><Icon name="pause" size={22} /></div>
          <button class="cbtn hover hot" title="Reprendre" onclick={(e) => act(e, () => onaction("resume", incomplete))}><Icon name="play" size={20} /></button>
        </div>
        <span class="eta paused-txt">En pause · {pct}%</span>
      {/if}
    </div>

    {#if !running && !paused}
      <span class="pill">
        {#if g.status === "error"}
          <Icon name="alert" size={12} />Manquant
        {:else if g.partial}
          <Icon name="check" size={12} />{partialLabel(g)}
        {:else}
          <Icon name="check" size={12} />Terminé
        {/if}
      </span>
    {/if}

    {#if selected}
      <span class="sel"><Icon name="check" size={14} /></span>
    {:else if g.items.length > 1}
      <span class="count">{g.items.length} {g.kind === "series" || g.kind === "anime" ? "ép." : "éléments"}</span>
    {/if}

    <div class="actions">
      <button class="glass-btn" title="Ouvrir le dossier" onclick={(e) => act(e, () => onaction("reveal", g.items[0]))}>
        <Icon name="folder" size={14} />
      </button>
    </div>

    {#if g.dlspeed > 0}
      <span class="live"><Icon name="down" size={11} />{fmtSpeed(g.dlspeed)}</span>
    {/if}
    {#if nextAir && !g.dlspeed}
      <span class="next-ep" title="Prochain épisode"><Icon name="calendar" size={11} />É{nextAir.n} · {nextAir.when}</span>
    {/if}
    {#if installed}
      <div class="installed-band" title="Installé : {installed.name}"><Icon name="check" size={13} />Installé</div>
    {/if}
    {#if format}<span class="console"><ConsoleBadge {format} platform={g.items.find((i) => i.platform)?.platform ?? ""} /></span>{/if}
    <div class="bar"><span style="width:{pct}%"></span></div>
  </div>

  <div class="meta">
    <div class="title" title={g.title}>{g.title}</div>
    <div class="sub">{[subtitle(g), fmtSize(g.size)].filter(Boolean).join(" · ")}</div>
    <div class="row">
      <span class="tags">
        <span class="kind k-{g.kind}"><Icon name={g.kind} size={11} />{KIND_LABEL[g.kind]}</span>
      </span>
      <span class="when">{when}</span>
    </div>
  </div>
</div>

<style>
  .card {
    position: relative;
    top: 0;
    display: flex;
    flex-direction: column;
    border-radius: var(--r);
    background: var(--surface);
    border: 1px solid var(--line);
    box-shadow: var(--shadow-sm);
    cursor: pointer;
    /* La carte monte avec « top » et non « transform » : un transform ferait passer les cartes
       empilées derrière (séries) par-dessus la carte */
    transition: top 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease;
    outline: none;
  }
  .card:hover,
  .card:focus-visible {
    top: -3px;
    border-color: hsl(var(--h) 70% 55% / 0.5);
    box-shadow: 0 18px 40px -18px hsl(var(--h) 70% 35% / 0.55);
  }
  .card.selected { border-color: var(--accent); box-shadow: 0 0 0 2.5px var(--accent), var(--shadow-sm); }
  .card.stack::before,
  .card.stack::after {
    content: "";
    position: absolute;
    inset: 0;
    border-radius: var(--r);
    background: var(--surface);
    border: 1px solid var(--line-strong);
    z-index: -1;
    transition: transform 0.18s ease;
  }
  .card.stack::before { transform: translateY(-5px) scale(0.95); opacity: 0.8; }
  .card.stack::after { transform: translateY(-10px) scale(0.9); opacity: 0.45; }
  .card.stack:hover::before { transform: translateY(-7px) scale(0.95); }
  .card.stack:hover::after { transform: translateY(-14px) scale(0.9); }

  .cover {
    position: relative;
    aspect-ratio: 2 / 3;
    border-radius: var(--r) var(--r) 0 0;
    overflow: hidden;
    background: hsl(var(--h) 40% 20%);
  }
  .art {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(120% 70% at 90% 0%, hsl(calc(var(--h) + 40) 85% 62% / 0.85), transparent 60%),
      radial-gradient(100% 70% at 0% 100%, hsl(calc(var(--h) - 30) 80% 35% / 0.9), transparent 70%),
      linear-gradient(160deg, hsl(var(--h) 60% 30%), hsl(calc(var(--h) + 60) 55% 14%));
    display: flex;
    align-items: flex-end;
    padding: 12px 12px 16px;
  }
  .watermark { position: absolute; right: -24px; top: 18%; color: white; opacity: 0.13; transform: rotate(-12deg); transition: transform 0.4s ease; }
  .cover-title {
    position: relative;
    font-size: var(--fs-cover);
    font-weight: 800;
    line-height: 1.08;
    letter-spacing: -0.02em;
    color: white;
    text-shadow: 0 2px 16px rgb(0 0 0 / 0.45);
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
    transition: opacity 0.35s ease, transform 0.5s ease, filter 0.25s;
  }
  img.loaded { opacity: 1; }
  .card:hover img { transform: scale(1.03); }
  .card.s-error .cover img, .card.s-error .art { filter: saturate(0.2) brightness(0.75); }
  .card.paused .cover img, .card.paused .art { filter: saturate(0.35) brightness(0.6); }
  .shade { position: absolute; inset: 0; background: linear-gradient(180deg, rgb(0 0 0 / 0.35), transparent 22%, transparent 75%, rgb(0 0 0 / 0.45)); pointer-events: none; }

  /* ---------- centre de l'affiche ---------- */
  .center { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; pointer-events: none; }
  .slot { position: relative; display: grid; place-items: center; width: 66px; height: 66px; flex: none; }
  .center { gap: 10px !important; }
  /* Pendant un téléchargement, l'affiche est assombrie pour bien lire la progression */
  .card.downloading .cover img, .card.downloading .art { filter: brightness(0.5) saturate(0.9); }
  .center .idle { transition: opacity 0.15s, scale 0.15s; }
  .card:hover .center .idle { opacity: 0; scale: 0.9; }
  .ring { position: relative; width: 66px; height: 66px; border-radius: 50%; background: rgb(8 10 16 / 0.62); backdrop-filter: blur(6px); box-shadow: 0 6px 24px rgb(0 0 0 / 0.4); }
  .ring svg { position: absolute; inset: 0; width: 100%; height: 100%; rotate: -90deg; }
  .ring circle { fill: none; stroke-width: 5; }
  .ring .track { stroke: rgb(255 255 255 / 0.15); }
  .ring .fill { stroke: #4aa8ff; stroke-linecap: round; transition: stroke-dashoffset 0.6s ease; filter: drop-shadow(0 0 4px #4aa8ff); }
  .pct { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; line-height: 1; color: white; font-size: 17px; font-weight: 800; font-variant-numeric: tabular-nums; letter-spacing: -0.02em; }
  .pct small { font-size: 10px; font-weight: 700; margin: 3px 0 0 1px; opacity: 0.8; }
  .eta { padding: 2px 8px; border-radius: 999px; background: rgb(8 10 16 / 0.62); backdrop-filter: blur(6px); color: white; font-size: 11px; font-weight: 700; white-space: nowrap; }
  .eta:empty { display: none; }
  .eta.paused-txt { background: #f59e0b; color: #1a1205; }
  .pdisk { display: grid; place-items: center; width: 58px; height: 58px; border-radius: 50%; background: #f59e0b; color: #1a1205; box-shadow: 0 6px 24px rgb(0 0 0 / 0.4); }
  .cbtn {
    position: absolute;
    left: 50%;
    top: 50%;
    translate: -50% -50%;
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    border: 0;
    background: rgb(255 255 255 / 0.94);
    color: #111;
    cursor: pointer;
    pointer-events: auto;
    opacity: 0;
    scale: 0.85;
    transition: opacity 0.15s, scale 0.15s;
    box-shadow: 0 8px 26px rgb(0 0 0 / 0.45);
  }
  .cbtn.hot { background: #f59e0b; color: #1a1205; }
  .card:hover .cbtn { opacity: 1; scale: 1; }
  .cbtn:hover { scale: 1.08 !important; }

  .next-ep { position: absolute; left: 8px; bottom: 12px; display: inline-flex; align-items: center; gap: 4px; padding: 3px 8px; border-radius: 999px; background: var(--accent); color: white; font-size: 11px; font-weight: 700; box-shadow: 0 4px 12px rgb(0 0 0 / 0.3); }
  .installed-band {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    height: 22px;
    background: rgb(16 185 129 / 0.92);
    color: white;
    font-size: 11.5px;
    font-weight: 800;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .pill,
  .count,
  .live {
    position: absolute;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 650;
    background: rgb(8 10 16 / 0.62);
    backdrop-filter: blur(8px);
    color: #eef0f6;
  }
  .pill { top: 8px; left: 8px; }
  .count { top: 8px; right: 8px; transition: opacity 0.15s; }
  .live { bottom: 12px; left: 8px; color: #9fd3ff; }
  .s-seeding .pill { color: #4ade9a; }
  .s-error .pill { color: #fb8a9c; }
  .card.paused .pill { background: #f59e0b; color: #1a1205; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: #4aa8ff; }
  .pulse { animation: pulse 1.4s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.35; transform: scale(0.8); } }

  .sel { position: absolute; top: 7px; right: 7px; display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; background: var(--accent); color: white; box-shadow: 0 2px 8px rgb(0 0 0 / 0.35); }


  .actions { position: absolute; top: 6px; right: 6px; display: flex; gap: 4px; opacity: 0; transform: translateY(-4px); transition: opacity 0.15s, transform 0.15s; }
  .card:hover .actions, .card:focus-visible .actions { opacity: 1; transform: none; }
  .card:hover .count, .card:hover .sel { opacity: 0; }
  .glass-btn { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 8px; border: 1px solid rgb(255 255 255 / 0.12); background: rgb(8 10 16 / 0.6); backdrop-filter: blur(8px); color: white; cursor: pointer; }
  .glass-btn:hover { background: rgb(8 10 16 / 0.85); }
  .glass-btn.hot { background: #f59e0b; color: #1a1205; border-color: transparent; }

  .bar { position: absolute; left: 0; right: 0; bottom: 0; height: 4px; background: rgb(0 0 0 / 0.45); }
  .bar span { display: block; height: 100%; background: linear-gradient(90deg, var(--accent), var(--accent-2)); box-shadow: 0 0 12px var(--accent); transition: width 0.6s ease; }
  .s-seeding .bar span, .card.complete .bar span { background: var(--ok); box-shadow: 0 0 10px var(--ok); }
  .s-error .bar span { background: var(--err); box-shadow: none; }
  .card.paused .bar span { background: #f59e0b; box-shadow: none; }

  .meta { padding: var(--meta-pad); display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .title { font-weight: 650; font-size: var(--fs-title); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { color: var(--muted); font-size: var(--fs-small); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 6px; margin-top: 4px; }
  .kind { display: inline-flex; align-items: center; gap: 4px; font-size: 10.5px; font-weight: 700; padding: 2px 6px 2px 5px; border-radius: 5px; background: var(--kc-bg); color: var(--kc); white-space: nowrap; }
  .tags { display: flex; gap: 4px; min-width: 0; overflow: hidden; }
  .console { position: absolute; right: 8px; bottom: 12px; }
  .inst { display: inline-flex; align-items: center; gap: 3px; font-size: 10.5px; font-weight: 700; padding: 2px 6px 2px 5px; border-radius: 5px; background: var(--ok); color: white; white-space: nowrap; }
  .k-film { --kc: #e0457b; --kc-bg: rgb(224 69 123 / 0.12); }
  .k-series { --kc: #3b82f6; --kc-bg: rgb(59 130 246 / 0.12); }
  .k-anime { --kc: #f97316; --kc-bg: rgb(249 115 22 / 0.13); }
  .k-game { --kc: #16a34a; --kc-bg: rgb(22 163 74 / 0.12); }
  .k-software { --kc: #0891b2; --kc-bg: rgb(8 145 178 / 0.12); }
  .k-book { --kc: #a16207; --kc-bg: rgb(161 98 7 / 0.12); }
  .k-audio { --kc: #9333ea; --kc-bg: rgb(147 51 234 / 0.12); }
  .k-other { --kc: var(--muted); --kc-bg: var(--chip); }
  .when { color: var(--muted); font-size: 11px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
