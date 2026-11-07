import { api, type EvRanking, type PricingOptions, type Vendor } from "../api";
import { clear, h, mount, money, percent, verdict } from "../ui";

interface Ctx {
  pricing: () => PricingOptions;
  onPricingChange: (cb: () => void) => void;
  openSimulator: (set_code: string, booster_code: string | null) => void;
}

type SortKey = "ratio" | "ev" | "price" | "sd" | "release_date" | "name";

export function rankingsView(ctx: Ctx): HTMLElement {
  const root = h("section", { class: "rankings" });
  const controls = h("div", { class: "rank-controls" });
  const tableEl = h("div", { class: "table-wrap" });
  let rows: EvRanking[] = [];
  let sortKey: SortKey = "ratio";
  let sortDesc = true;
  let query = "";
  let category = "";
  let minPrice = 0;
  let request = 0;

  const search = h("input", {
    type: "search",
    class: "search",
    placeholder: "Filter by product or set…",
    onInput: () => {
      query = search.value.trim().toLowerCase();
      renderTable();
    },
  });
  const categorySelect = h("select", {
    onChange: () => {
      category = categorySelect.value;
      renderTable();
    },
  });
  const minPriceInput = h("input", {
    type: "number",
    min: "0",
    step: "1",
    placeholder: "Min price",
    class: "min-price",
    onInput: () => {
      minPrice = Number(minPriceInput.value) || 0;
      renderTable();
    },
  });
  mount(controls, search, categorySelect, minPriceInput);

  async function load(): Promise<void> {
    const id = ++request;
    tableEl.replaceChildren(h("p", { class: "muted" }, "Computing expected value for every sealed product…"));
    try {
      const result = await api.ev(ctx.pricing());
      if (id !== request) return;
      rows = result;
      renderCategories();
      renderTable();
    } catch (error) {
      if (id !== request) return;
      tableEl.replaceChildren(h("p", { class: "error" }, error instanceof Error ? error.message : String(error)));
    }
  }

  function renderCategories(): void {
    const categories = [...new Set(rows.map((r) => r.category ?? "").filter(Boolean))].sort();
    clear(categorySelect);
    mount(categorySelect, 
      h("option", { value: "" }, "All categories"),
      ...categories.map((c) => h("option", { value: c }, c.replace(/_/g, " "))),
    );
    categorySelect.value = category;
  }

  function sorted(): EvRanking[] {
    const filtered = rows.filter(
      (r) =>
        (!category || r.category === category) &&
        r.price >= minPrice &&
        (!query || `${r.name} ${r.set_name} ${r.set_code}`.toLowerCase().includes(query)),
    );
    const dir = sortDesc ? -1 : 1;
    return filtered.sort((a, b) => {
      const av = a[sortKey];
      const bv = b[sortKey];
      if (typeof av === "number" && typeof bv === "number") return (av - bv) * dir;
      return String(av ?? "").localeCompare(String(bv ?? "")) * dir;
    });
  }

  function header(label: string, key: SortKey, numeric = false): HTMLElement {
    const active = key === sortKey;
    return h(
      "th",
      { class: numeric ? "num" : "" },
      h(
        "button",
        {
          type: "button",
          class: `sort${active ? " active" : ""}`,
          onClick: () => {
            if (active) sortDesc = !sortDesc;
            else {
              sortKey = key;
              sortDesc = key !== "name";
            }
            renderTable();
          },
        },
        label,
        active ? (sortDesc ? " ▾" : " ▴") : "",
      ),
    );
  }

  function renderTable(): void {
    const list = sorted();
    clear(tableEl);
    if (list.length === 0) {
      mount(tableEl, h("p", { class: "muted" }, "No products match."));
      return;
    }
    mount(tableEl, 
      h("p", { class: "muted small" }, `${list.length} products priced at ${vendorLabel(ctx.pricing().vendor)}`,
        ctx.pricing().floor > 0 ? `, cards under ${money(ctx.pricing().floor)} counted as $0.` : "."),
      h(
        "table",
        null,
        h(
          "thead",
          null,
          h(
            "tr",
            null,
            header("Product", "name"),
            header("Released", "release_date"),
            header("Price", "price", true),
            header("EV", "ev", true),
            header("EV / price", "ratio", true),
            header("Std. dev.", "sd", true),
            h("th", null, "Verdict"),
          ),
        ),
        h(
          "tbody",
          null,
          ...list.slice(0, 500).map((r) => {
            const v = verdict(r.ratio, null, r.sd, r.price);
            return h(
              "tr",
              null,
              h(
                "td",
                null,
                h("button", { type: "button", class: "link product-link", onClick: () => ctx.openSimulator(r.booster_set_code ?? r.set_code, r.booster_code) }, r.name),
                h("span", { class: "muted small" }, `${r.set_name} · ${(r.subtype ?? r.category ?? "").replace(/_/g, " ")}`),
              ),
              h("td", null, r.release_date ?? "—"),
              h("td", { class: "num" }, money(r.price)),
              h("td", { class: "num" }, money(r.ev)),
              h("td", { class: `num ratio tone-${v.tone}` }, percent(r.ratio)),
              h("td", { class: "num" }, money(r.sd)),
              h("td", null, h("span", { class: `badge tone-${v.tone}` }, v.label)),
            );
          }),
        ),
      ),
      list.length > 500 ? h("p", { class: "muted small" }, "Showing the first 500 rows; narrow the filter to see more.") : null,
    );
  }

  function vendorLabel(vendor: Vendor): string {
    return { tcgplayer: "TCGplayer market", cardkingdom: "Card Kingdom", manapool: "ManaPool" }[vendor];
  }

  ctx.onPricingChange(() => void load());
  mount(root, 
    h("h2", null, "Sealed product expected value"),
    h(
      "p",
      { class: "muted" },
      "EV is the sum over every pack in the product of each card's probability times its single price. Ratios above 100% mean the singles are worth more than the sealed price on average; most product sits well below that.",
    ),
    controls,
    tableEl,
  );
  void load();
  return root;
}
