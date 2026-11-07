export type Vendor = "tcgplayer" | "cardkingdom" | "manapool";

export const VENDORS: { id: Vendor; label: string; short: string }[] = [
  { id: "tcgplayer", label: "TCGplayer market", short: "TCG" },
  { id: "cardkingdom", label: "Card Kingdom", short: "CK" },
  { id: "manapool", label: "ManaPool", short: "MP" },
];

export type Prices = Partial<Record<Vendor, number>>;

/** Price floor in dollars; cards below it count as $0 (bulk is rarely sellable). */
export const FLOORS = [0, 0.25, 0.5, 1, 2, 5] as const;

export interface PricingOptions {
  vendor: Vendor;
  floor: number;
}

export interface Status {
  ready: boolean;
  busy: boolean;
  message: string;
  error: string | null;
  mtgjson_version: string | null;
  prices_updated_at: string | null;
  booster_count: number;
  product_count: number;
}

export interface ProductSummary {
  uuid: string;
  name: string;
  count: number;
  prices: Prices;
}

export interface BoosterListing {
  set_code: string;
  set_name: string;
  release_date: string | null;
  set_type: string | null;
  keyrune_code: string | null;
  booster_code: string;
  name: string;
  card_count: number;
  pack: ProductSummary | null;
  box: ProductSummary | null;
}

export interface OpenedCard {
  uuid: string;
  name: string;
  set_code: string;
  number: string;
  rarity: string | null;
  foil: boolean;
  sheet: string;
  scryfall_id: string | null;
  image: string | null;
  prices: Prices;
}

export interface OpenedPack {
  cards: OpenedCard[];
  value: Prices;
}

export interface OpenResponse {
  set_code: string;
  booster_code: string;
  packs: OpenedPack[];
}

export interface HistogramBin {
  from: number;
  to: number;
  count: number;
}

export interface Simulated {
  samples: number;
  mean: number;
  variance: number;
  sd: number;
  median: number;
  min: number;
  max: number;
  break_even: number | null;
  histogram: HistogramBin[];
}

export interface StatsResponse {
  vendor: Vendor;
  floor: number;
  set_code: string;
  booster_code: string;
  card_count: number;
  pack_price: number | null;
  theoretical: {
    ev: number;
    variance: number;
    sd: number;
    priced_cards: number;
    unpriced_cards: number;
  };
  simulated: Simulated;
  box: {
    uuid: string;
    name: string;
    count: number;
    price: number | null;
    ev: number;
    sd: number;
    simulated: Simulated;
  } | null;
}

export interface EvRanking {
  uuid: string;
  name: string;
  set_code: string;
  set_name: string;
  category: string | null;
  subtype: string | null;
  release_date: string | null;
  ev: number;
  sd: number;
  price: number;
  ratio: number;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
  });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // keep the status text
    }
    throw new Error(message);
  }
  return (await response.json()) as T;
}

export const api = {
  status: () => request<Status>("/api/status"),
  products: () => request<BoosterListing[]>("/api/products"),
  open: (set_code: string, booster_code: string, count: number, floor: number) =>
    request<OpenResponse>("/api/open", {
      method: "POST",
      body: JSON.stringify({ set_code, booster_code, count, floor }),
    }),
  stats: (set_code: string, booster_code: string, pricing: PricingOptions, samples = 1000) =>
    request<StatsResponse>(
      `/api/stats?${new URLSearchParams({
        set_code,
        booster_code,
        vendor: pricing.vendor,
        floor: String(pricing.floor),
        samples: String(samples),
      })}`,
    ),
  ev: (pricing: PricingOptions) =>
    request<EvRanking[]>(`/api/ev?${new URLSearchParams({ vendor: pricing.vendor, floor: String(pricing.floor) })}`),
  refresh: (catalog = false) =>
    request<unknown>("/api/refresh", { method: "POST", body: JSON.stringify({ catalog }) }),
};
