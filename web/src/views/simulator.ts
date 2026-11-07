import { api, type BoosterListing, type OpenedCard, type OpenedPack, type PricingOptions, type StatsResponse, type Vendor } from "../api";
import { prefs, type HistoryEntry } from "../state";
import { cents, clear, h, mount, money, percent, verdict } from "../ui";

interface Ctx {
  listings: BoosterListing[];
  pricing: () => PricingOptions;
  onPricingChange: (cb: () => void) => void;
}

function rank(listing: BoosterListing): number {
  if (listing.booster_code.includes("arena")) return 2;
  return listing.pack ? 0 : 1;
}

export function simulatorView(ctx: Ctx): HTMLElement {
  const root = h("section", { class: "simulator" });
  const pickerEl = h("div", { class: "picker" });
  const actionsEl = h("div", { class: "actions" });
  const statsSummary = h("summary", { class: "stats-summary" }, "Expected value");
  const statsEl = h("div", { class: "stats-body" });
  // Collapsed on phones so the picker and open buttons come first.
  const statsPanel = h("details", { class: "stats", open: window.matchMedia("(min-width: 900px)").matches }, statsSummary, statsEl);
  const packsEl = h("div", { class: "packs" });
  const historyEl = h("div", { class: "history" });

  let selected: BoosterListing | null = null;
  let lastCount = 1;
  let stats: StatsResponse | null = null;
  let statsRequest = 0;
  let packs: OpenedPack[] = [];
  let opening = false;

  // Physical products with a sealed listing first; Arena-only boosters last.
  const listings = [...ctx.listings].sort((a, b) => rank(a) - rank(b));
  const byKey = new Map(listings.map((l) => [`${l.set_code}/${l.booster_code}`, l]));
  const remembered = prefs.selection;
  if (remembered) selected = byKey.get(`${remembered.set_code}/${remembered.booster_code}`) ?? null;
  if (!selected) selected = listings.find((l) => l.pack?.prices[ctx.pricing().vendor] !== undefined) ?? listings[0] ?? null;

  function select(listing: BoosterListing): void {
    selected = listing;
    prefs.selection = { set_code: listing.set_code, booster_code: listing.booster_code };
    packs = [];
    stats = null;
    renderPicker();
    renderActions();
    renderPacks();
    void loadStats();
  }

  async function loadStats(): Promise<void> {
    if (!selected) return;
    const id = ++statsRequest;
    renderStats("loading");
    try {
      const result = await api.stats(selected.set_code, selected.booster_code, ctx.pricing());
      if (id !== statsRequest) return;
      stats = result;
      renderStats();
    } catch (error) {
      if (id !== statsRequest) return;
      renderStats(error instanceof Error ? error.message : String(error));
    }
  }

  async function open(count: number): Promise<void> {
    if (!selected || opening) return;
    opening = true;
    lastCount = count;
    renderActions();
    try {
      const result = await api.open(selected.set_code, selected.booster_code, count, ctx.pricing().floor);
      packs = result.packs;
      record(selected, result.packs);
      renderPacks();
      renderHistory();
    } catch (error) {
      packsEl.replaceChildren(h("p", { class: "error" }, error instanceof Error ? error.message : String(error)));
    } finally {
      opening = false;
      renderActions();
    }
  }

  function record(listing: BoosterListing, opened: OpenedPack[]): void {
    const value: HistoryEntry["value"] = {};
    const cost: HistoryEntry["cost"] = {};
    for (const vendor of ["tcgplayer", "cardkingdom", "manapool"] as Vendor[]) {
      const total = opened.reduce((sum, p) => sum + (p.value[vendor] ?? 0), 0);
      if (opened.some((p) => p.value[vendor] !== undefined)) value[vendor] = total;
      const price = listing.pack?.prices[vendor];
      if (price !== undefined) cost[vendor] = price * opened.length;
    }
    const vendor = ctx.pricing().vendor;
    let best: HistoryEntry["best"] = { name: "", cents: null, foil: false };
    for (const card of opened.flatMap((p) => p.cards)) {
      const price = card.prices[vendor] ?? 0;
      if (best.cents === null || price > best.cents) best = { name: card.name, cents: price, foil: card.foil };
    }
    prefs.pushHistory({
      at: new Date().toISOString(),
      set_code: listing.set_code,
      booster_code: listing.booster_code,
      name: listing.name,
      packs: opened.length,
      value,
      cost,
      best,
    });
  }

  // ---- picker -------------------------------------------------------------

  const search = h("input", {
    class: "search",
    type: "search",
    placeholder: "Search sets or booster types…",
    autocomplete: "off",
    onInput: () => renderResults(),
    onKeydown: (event) => {
      if (event.key === "Escape") {
        search.value = "";
        renderResults();
        search.blur();
      }
      if (event.key === "Enter") {
        const first = results.querySelector<HTMLButtonElement>("button");
        first?.click();
      }
    },
  });
  const results = h("div", { class: "results", hidden: true });

  function matches(listing: BoosterListing, query: string): boolean {
    const haystack = `${listing.set_name} ${listing.set_code} ${listing.name} ${listing.booster_code}`.toLowerCase();
    return query.split(/\s+/).every((word) => haystack.includes(word));
  }

  function renderResults(): void {
    const query = search.value.trim().toLowerCase();
    const found = listings.filter((l) => matches(l, query)).slice(0, 60);
    results.hidden = found.length === 0;
    results.replaceChildren(
      ...found.map((listing) => {
        const price = listing.pack?.prices[ctx.pricing().vendor];
        return h(
          "button",
          {
            type: "button",
            class: `result${listing === selected ? " selected" : ""}`,
            onClick: () => {
              search.value = "";
              results.hidden = true;
              select(listing);
            },
          },
          h("span", { class: "result-name" }, listing.name),
          h("span", { class: "result-meta" }, `${listing.set_code} · ${listing.release_date ?? "?"} · ${listing.card_count} cards`),
          h("span", { class: "result-price" }, price !== undefined ? cents(price) : ""),
        );
      }),
    );
  }

  search.addEventListener("focus", () => renderResults());
  document.addEventListener("click", (event) => {
    if (!pickerEl.contains(event.target as Node)) results.hidden = true;
  });

  function renderPicker(): void {
    clear(pickerEl);
    mount(pickerEl, 
      h(
        "div",
        { class: "selected-booster" },
        selected
          ? [
              h("h2", null, selected.name),
              h(
                "p",
                { class: "muted" },
                `${selected.set_name} (${selected.set_code}) · ${selected.release_date ?? "unknown date"} · ${selected.card_count} cards per pack`,
              ),
            ]
          : h("h2", null, "Pick a booster"),
      ),
      search,
      results,
    );
  }

  // ---- actions ------------------------------------------------------------

  function renderActions(): void {
    clear(actionsEl);
    if (!selected) return;
    const vendor = ctx.pricing().vendor;
    const price = selected.pack?.prices[vendor];
    const boxCount = selected.box?.count ?? 0;
    const button = (label: string, count: number, hint?: string) =>
      h(
        "button",
        { type: "button", class: "open-button", disabled: opening, onClick: () => void open(count) },
        label,
        hint ? h("small", null, hint) : null,
      );
    mount(actionsEl, 
      button("Open 1", 1, "R"),
      button("Open 3", 3),
      button("Open 6", 6),
      boxCount >= 2 && boxCount <= 36 ? button(`Open box`, boxCount, `${boxCount} packs`) : null,
      h(
        "span",
        { class: "pack-price" },
        price !== undefined ? ["Pack price ", h("strong", null, cents(price))] : "No sealed price at this vendor",
      ),
    );
  }

  // ---- packs --------------------------------------------------------------

  function rarityClass(card: OpenedCard): string {
    return `rarity-${(card.rarity ?? "common").toLowerCase().replace(/\s+/g, "-")}`;
  }

  function cardEl(card: OpenedCard, vendor: Vendor): HTMLElement {
    const price = card.prices[vendor];
    const img = card.image
      ? h("img", { src: card.image, alt: card.name, loading: "lazy", decoding: "async", width: 488, height: 680 })
      : h("div", { class: "card-missing" }, card.name);
    return h(
      "figure",
      { class: `card ${rarityClass(card)}${card.foil ? " foil" : ""}`, title: `${card.name} (${card.sheet})` },
      h("div", { class: "card-art" }, img, card.foil ? h("span", { class: "foil-badge" }, "foil") : null),
      h(
        "figcaption",
        null,
        h("span", { class: "card-name" }, card.name),
        h("span", { class: "card-price" }, price !== undefined ? cents(price) : "—"),
      ),
    );
  }

  function renderPacks(): void {
    clear(packsEl);
    if (!selected) return;
    if (packs.length === 0) {
      mount(packsEl, 
        h(
          "p",
          { class: "empty" },
          "Open some packs. Press ",
          h("kbd", null, "R"),
          " to repeat the last opening.",
        ),
      );
      return;
    }
    const vendor = ctx.pricing().vendor;
    const price = selected.pack?.prices[vendor];
    const total = packs.reduce((sum, p) => sum + (p.value[vendor] ?? 0), 0);
    const spent = price !== undefined ? price * packs.length : null;
    mount(packsEl, 
      h(
        "div",
        { class: "packs-summary" },
        h("strong", null, `${packs.length} pack${packs.length === 1 ? "" : "s"}`),
        " worth ",
        h("strong", null, cents(total)),
        spent !== null ? [" for ", h("strong", null, cents(spent)), " ", delta(total, spent)] : null,
      ),
    );
    packs.forEach((pack, index) => {
      const value = pack.value[vendor];
      const sorted = [...pack.cards].sort((a, b) => (b.prices[vendor] ?? 0) - (a.prices[vendor] ?? 0));
      mount(packsEl, 
        h(
          "article",
          { class: "pack" },
          h(
            "header",
            null,
            h("h3", null, `Pack ${index + 1}`),
            h(
              "span",
              { class: "pack-value" },
              value !== undefined ? cents(value) : "—",
              price !== undefined && value !== undefined ? delta(value, price) : null,
            ),
          ),
          h("div", { class: "card-grid" }, ...sorted.map((card) => cardEl(card, vendor))),
        ),
      );
    });
  }

  function delta(value: number, cost: number): HTMLElement {
    const diff = value - cost;
    return h(
      "span",
      { class: `delta ${diff >= 0 ? "up" : "down"}` },
      `${diff >= 0 ? "+" : "−"}${cents(Math.abs(diff))}`,
    );
  }

  // ---- stats --------------------------------------------------------------

  function histogram(stat: StatsResponse["simulated"], price: number | null): SVGElement {
    const width = 320;
    const height = 110;
    const pad = 4;
    const bins = stat.histogram;
    const max = Math.max(1, ...bins.map((b) => b.count));
    // The server's last bin is an overflow bin that runs out to the max
    // value, so bins are drawn at equal widths rather than to scale.
    const slot = (width - pad * 2) / Math.max(bins.length, 1);
    const lo = bins[0]?.from ?? 0;
    const cap = bins[bins.length - 1]?.from ?? lo;
    const x = (v: number) => pad + ((v - lo) / Math.max(cap - lo, 1e-9)) * slot * (bins.length - 1);
    const ns = "http://www.w3.org/2000/svg";
    const svg = document.createElementNS(ns, "svg");
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    svg.setAttribute("class", "histogram");
    svg.setAttribute("role", "img");
    svg.setAttribute("aria-label", "Distribution of simulated pack values");
    bins.forEach((bin, i) => {
      const rect = document.createElementNS(ns, "rect");
      const barHeight = (bin.count / max) * (height - 18);
      rect.setAttribute("x", (pad + slot * i).toFixed(1));
      rect.setAttribute("width", Math.max(1, slot - 1).toFixed(1));
      rect.setAttribute("y", (height - 14 - barHeight).toFixed(1));
      rect.setAttribute("height", barHeight.toFixed(1));
      rect.setAttribute("class", price !== null && bin.to <= price ? "bar below" : "bar");
      const title = document.createElementNS(ns, "title");
      title.textContent = `${money(bin.from)} – ${money(bin.to)}: ${bin.count} packs`;
      rect.appendChild(title);
      svg.appendChild(rect);
    });
    if (price !== null && price >= lo && price <= cap) {
      const line = document.createElementNS(ns, "line");
      line.setAttribute("x1", x(price).toFixed(1));
      line.setAttribute("x2", x(price).toFixed(1));
      line.setAttribute("y1", "0");
      line.setAttribute("y2", String(height - 14));
      line.setAttribute("class", "price-line");
      svg.appendChild(line);
    }
    const label = (text: string, xPos: number, anchor: string) => {
      const t = document.createElementNS(ns, "text");
      t.setAttribute("x", String(xPos));
      t.setAttribute("y", String(height - 2));
      t.setAttribute("text-anchor", anchor);
      t.setAttribute("class", "axis");
      t.textContent = text;
      svg.appendChild(t);
    };
    label(money(lo), pad, "start");
    label(`≥ ${money(cap)}`, width - pad, "end");
    return svg;
  }

  function statRow(label: string, value: string, hint?: string): HTMLElement {
    return h("div", { class: "stat", title: hint ?? "" }, h("dt", null, label), h("dd", null, value));
  }

  function renderStats(state?: "loading" | string): void {
    clear(statsEl);
    if (!selected) return;
    if (state === "loading") {
      mount(statsEl, h("p", { class: "muted" }, "Computing expected value…"));
      return;
    }
    if (typeof state === "string") {
      mount(statsEl, h("p", { class: "error" }, state));
      return;
    }
    if (!stats) return;
    const s = stats;
    const price = s.pack_price;
    const ratio = price ? s.theoretical.ev / price : null;
    const v = verdict(ratio, s.simulated.break_even, s.simulated.sd, price);
    statsSummary.replaceChildren(
      "Expected value ",
      h("span", { class: `badge tone-${v.tone}` }, v.label, ratio !== null ? ` · ${percent(ratio)}` : ""),
    );
    mount(statsEl, 
      h(
        "div",
        { class: `verdict tone-${v.tone}` },
        h("span", { class: "verdict-label" }, v.label),
        ratio !== null ? h("span", { class: "verdict-ratio" }, percent(ratio)) : null,
        h("p", null, v.blurb),
      ),
      h(
        "dl",
        { class: "stat-grid" },
        statRow("Expected value", money(s.theoretical.ev), "Closed-form mean of the pack from sheet weights and single prices"),
        statRow("Pack price", money(price)),
        statRow("Std. deviation", money(s.theoretical.sd)),
        statRow("Median (sim.)", money(s.simulated.median), `${s.simulated.samples} simulated packs`),
        statRow("Break-even", percent(s.simulated.break_even), "Share of simulated packs worth at least the pack price"),
        statRow("Sim. mean", money(s.simulated.mean)),
        statRow("Best sim. pack", money(s.simulated.max)),
        statRow("Worst sim. pack", money(s.simulated.min)),
      ),
      histogram(s.simulated, price),
      h(
        "p",
        { class: "muted small" },
        `${s.theoretical.priced_cards} priced cards`,
        s.theoretical.unpriced_cards > 0 ? `, ${s.theoretical.unpriced_cards} without a price (counted as $0)` : "",
        `. Prices: ${vendorLabel(s.vendor)}`,
        s.floor > 0 ? `, cards under ${money(s.floor)} counted as $0` : "",
        ".",
      ),
    );
    if (s.box) {
      const b = s.box;
      const boxRatio = b.price ? b.ev / b.price : null;
      const bv = verdict(boxRatio, b.simulated.break_even, b.sd, b.price);
      mount(statsEl, 
        h(
          "section",
          { class: "box-stats" },
          h("h3", null, b.name),
          h(
            "div",
            { class: `verdict compact tone-${bv.tone}` },
            h("span", { class: "verdict-label" }, bv.label),
            boxRatio !== null ? h("span", { class: "verdict-ratio" }, percent(boxRatio)) : null,
          ),
          h(
            "dl",
            { class: "stat-grid" },
            statRow("Expected value", money(b.ev), `${b.count} packs`),
            statRow("Box price", money(b.price)),
            statRow("Std. deviation", money(b.sd)),
            statRow("Break-even", percent(b.simulated.break_even), `${b.simulated.samples} simulated boxes`),
          ),
        ),
      );
    }
  }

  function vendorLabel(vendor: Vendor): string {
    return { tcgplayer: "TCGplayer market", cardkingdom: "Card Kingdom", manapool: "ManaPool" }[vendor];
  }

  // ---- history ------------------------------------------------------------

  function renderHistory(): void {
    clear(historyEl);
    const entries = prefs.history;
    if (entries.length === 0) return;
    const vendor = ctx.pricing().vendor;
    const totalPacks = entries.reduce((sum, e) => sum + e.packs, 0);
    const totalValue = entries.reduce((sum, e) => sum + (e.value[vendor] ?? 0), 0);
    const totalCost = entries.reduce((sum, e) => sum + (e.cost[vendor] ?? 0), 0);
    mount(historyEl, 
      h(
        "header",
        null,
        h("h3", null, "Session"),
        h(
          "span",
          { class: "muted" },
          `${totalPacks} pack${totalPacks === 1 ? "" : "s"} · ${cents(totalValue)} pulled`,
          totalCost > 0 ? [" · ", cents(totalCost), " spent ", delta(totalValue, totalCost)] : null,
        ),
        h("button", { type: "button", class: "link", onClick: () => (prefs.clearHistory(), renderHistory()) }, "Clear"),
      ),
      h(
        "ul",
        null,
        ...entries.slice(0, 12).map((e) =>
          h(
            "li",
            null,
            h("span", { class: "history-name" }, `${e.packs}× ${e.name}`),
            h("span", { class: "history-best" }, e.best.name ? `best: ${e.best.name}${e.best.foil ? " (foil)" : ""}` : ""),
            h(
              "span",
              { class: "history-value" },
              cents(e.value[vendor] ?? 0),
              e.cost[vendor] !== undefined ? delta(e.value[vendor] ?? 0, e.cost[vendor]) : null,
            ),
          ),
        ),
      ),
    );
  }

  // ---- wiring -------------------------------------------------------------

  document.addEventListener("keydown", (event) => {
    if (!root.isConnected) return;
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "SELECT" || target.tagName === "TEXTAREA")) return;
    if (event.key === "r" || event.key === "R") {
      event.preventDefault();
      void open(lastCount);
    }
    if (event.key === "/") {
      event.preventDefault();
      search.focus();
    }
  });

  ctx.onPricingChange(() => {
    renderActions();
    renderPacks();
    renderHistory();
    void loadStats();
  });

  mount(root, 
    h("div", { class: "main-column" }, pickerEl, actionsEl, packsEl, historyEl),
    statsPanel,
  );
  renderPicker();
  renderActions();
  renderPacks();
  renderHistory();
  void loadStats();
  return root;
}
