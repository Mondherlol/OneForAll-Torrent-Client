<script>
  // Onglet « Sources » : qui partage ce torrent ? Pourquoi il est bloqué ?
  import Icon from "./Icon.svelte";
  import { getSources } from "./api.js";
  import { fmtSize, fmtSpeed, isPaused } from "./model.js";

  let { items } = $props();

  const single = $derived(items.length === 1);
  const t = $derived(items[0]);
  let data = $state(null);
  let err = $state(null);

  $effect(() => {
    if (!single) return;
    const hash = t.hash;
    let stop = false;
    const load = async () => {
      try {
        const d = await getSources(hash);
        if (!stop) (data = d), (err = null);
      } catch (e) {
        if (!stop) err = String(e);
      }
    };
    load();
    const id = setInterval(() => !document.hidden && load(), 3000);
    return () => ((stop = true), clearInterval(id));
  });

  // Diagnostic en clair
  function diagnose(it, d) {
    if (it.status === "error") return { level: "bad", text: "Les fichiers sont introuvables sur le disque (déplacés ou supprimés)." };
    if (it.progress >= 1) return { level: "ok", text: "Téléchargement terminé." };
    if (isPaused(it.status)) return { level: "warn", text: "Ce torrent est en pause : il ne cherche pas de sources." };
    const trackersDown = d && d.trackers.length > 0 && d.trackers.every((x) => !x.ok);
    if (it.seeds === 0 && it.peers === 0 && trackersDown)
      return { level: "bad", text: "Aucun tracker ne répond : impossible de trouver des sources. Le torrent est peut-être mort ou le tracker en panne." };
    if (it.seeds_total === 0 && it.availability < 1)
      return {
        level: "bad",
        text: `Personne ne partage ce torrent en entier pour l'instant (0 seeder). ${it.peers_total > 0 ? `${it.peers_total} personne(s) le téléchargent aussi, mais il manque des morceaux à tout le monde.` : ""} Il risque de rester bloqué.`,
      };
    if (it.seeds === 0) return { level: "warn", text: `${it.seeds_total} seeder(s) existent, mais aucun n'est connecté pour l'instant. Patiente un peu…` };
    if (it.dlspeed === 0) return { level: "warn", text: `Connecté à ${it.seeds} seeder(s), mais rien ne descend pour l'instant.` };
    return { level: "ok", text: `Connecté à ${it.seeds} seeder(s) · ${fmtSpeed(it.dlspeed)}` };
  }

  const diag = $derived(single ? diagnose(t, data) : null);
  const shortUrl = (u) => u.replace(/^(https?|udp):\/\//, "").replace(/\/announce.*$/, "").replace(/\/[a-f0-9]{16,}.*$/i, "");
</script>

<div class="sources">
  {#if single}
    <div class="diag {diag.level}">
      <Icon name={diag.level === "ok" ? "check" : "alert"} size={16} />
      <span>{diag.text}</span>
    </div>

    <div class="stats">
      <div><span>Seeders</span><b>{t.seeds}<em> connectés</em></b><small>{t.seeds_total >= 0 ? `${t.seeds_total} dans l'essaim` : "—"}</small></div>
      <div><span>Leechers</span><b>{t.peers}<em> connectés</em></b><small>{t.peers_total >= 0 ? `${t.peers_total} dans l'essaim` : "—"}</small></div>
      <div><span>Disponibilité</span><b>{t.availability >= 0 ? t.availability.toFixed(2).replace(".", ",") : "—"}</b><small>{t.availability >= 1 ? "copie complète accessible" : "il manque des morceaux"}</small></div>
      <div><span>Ratio</span><b>{t.ratio.toFixed(2).replace(".", ",")}</b><small>↑ {fmtSpeed(t.upspeed)}</small></div>
    </div>

    {#if err}<p class="muted">{err}</p>{/if}

    {#if data}
      <h4>Trackers</h4>
      <ul class="trackers">
        {#each data.trackers as tr}
          <li>
            <span class="dot" class:ok={tr.ok}></span>
            <span class="url" title={tr.url}>{shortUrl(tr.url)}</span>
            <span class="st" class:ok={tr.ok}>{tr.status}</span>
            {#if tr.seeds >= 0}<span class="nums">↑ {tr.seeds} · ↓ {tr.leeches}</span>{/if}
            {#if tr.message}<span class="msg" title={tr.message}>{tr.message}</span>{/if}
          </li>
        {:else}
          <li class="muted">Aucun tracker (DHT uniquement)</li>
        {/each}
      </ul>

      <h4>Pairs connectés · {data.peers.length}</h4>
      {#if data.peers.length}
        <ul class="peers">
          {#each data.peers as p}
            <li>
              <span class="tag" class:seed={p.seed}>{p.seed ? "Seeder" : `${Math.floor(p.progress * 100)} %`}</span>
              <span class="client" title={p.ip}>{p.client || p.ip}</span>
              <span class="country">{p.country}</span>
              <span class="speed">{p.dl > 0 ? `↓ ${fmtSpeed(p.dl)}` : ""} {p.up > 0 ? `↑ ${fmtSpeed(p.up)}` : ""}</span>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="muted">Aucun pair connecté pour le moment.</p>
      {/if}
    {:else if !err}
      <p class="muted">Chargement…</p>
    {/if}
  {:else}
    <!-- Groupe : un résumé par torrent -->
    <ul class="group">
      {#each items as it (it.hash)}
        {@const d = diagnose(it, null)}
        <li>
          <span class="dot {d.level}"></span>
          <span class="gname" title={it.name}>{it.episode != null ? `É${it.episode}` : it.title}</span>
          <span class="gtxt" title={d.text}>{d.text}</span>
          <span class="nums">↑ {it.seeds}/{it.seeds_total} · ↓ {it.peers}/{it.peers_total}</span>
        </li>
      {/each}
    </ul>
    <p class="muted">« connectés / dans l'essaim » — ouvre un épisode seul pour voir le détail des trackers et des pairs.</p>
  {/if}
</div>

<style>
  .sources { display: flex; flex-direction: column; gap: 10px; overflow-y: auto; min-height: 0; flex: 1; padding-right: 2px; }
  .diag { display: flex; gap: 10px; align-items: flex-start; padding: 10px 12px; border-radius: 10px; font-size: 13px; line-height: 1.45; }
  .diag.ok { background: rgb(16 185 129 / 0.1); color: var(--ok); }
  .diag.warn { background: rgb(245 158 11 / 0.12); color: #d97706; }
  .diag.bad { background: var(--err-soft); color: var(--err); }
  .diag span { color: var(--text); }
  .stats { display: grid; grid-template-columns: repeat(4, 1fr); gap: 6px; }
  .stats div { display: flex; flex-direction: column; gap: 2px; padding: 9px 10px; border-radius: 10px; background: var(--surface); border: 1px solid var(--line); min-width: 0; }
  .stats span { font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .stats b { font-size: 18px; font-variant-numeric: tabular-nums; }
  .stats em { font-style: normal; font-size: 11px; font-weight: 500; color: var(--muted); }
  .stats small { font-size: 11px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  h4 { margin: 8px 2px 0; font-size: 11.5px; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted); }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
  li { display: flex; align-items: center; gap: 8px; min-height: 32px; padding: 4px 8px; border-radius: 8px; font-size: 12.5px; }
  li:hover { background: var(--chip); }
  .trackers li { flex-wrap: wrap; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--err); flex: none; }
  .dot.ok { background: var(--ok); }
  .dot.warn { background: #f59e0b; }
  .dot.bad { background: var(--err); }
  .url { font-family: ui-monospace, "Cascadia Code", monospace; font-size: 12px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 220px; }
  .st { font-size: 11.5px; font-weight: 700; color: var(--err); }
  .st.ok { color: var(--ok); }
  .nums { margin-left: auto; font-size: 11.5px; color: var(--muted); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .msg { flex-basis: 100%; padding-left: 16px; font-size: 11.5px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tag { flex: none; min-width: 52px; text-align: center; padding: 2px 6px; border-radius: 5px; background: var(--chip); font-size: 11px; font-weight: 700; color: var(--text-2); }
  .tag.seed { background: rgb(16 185 129 / 0.14); color: var(--ok); }
  .client { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .country { font-size: 11.5px; color: var(--muted); white-space: nowrap; }
  .speed { font-size: 11.5px; color: var(--accent); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .gname { flex: none; font-weight: 700; min-width: 40px; }
  .gtxt { flex: 1; min-width: 0; font-size: 11.5px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .muted { margin: 4px 2px; font-size: 12px; color: var(--muted); }
</style>
