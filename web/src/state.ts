import type { Vendor } from "./api";

const KEYS = {
  vendor: "packrat.vendor",
  floor: "packrat.floor",
  selection: "packrat.selection",
  showArena: "packrat.showArena",
  history: "packrat.history",
} as const;

export interface Selection {
  set_code: string;
  booster_code: string;
}

export interface HistoryEntry {
  at: string;
  set_code: string;
  booster_code: string;
  name: string;
  packs: number;
  /** Total pack value in cents at the time, per vendor. */
  value: Partial<Record<Vendor, number>>;
  /** Sealed pack price in cents at the time, per vendor. */
  cost: Partial<Record<Vendor, number>>;
  best: { name: string; cents: number | null; foil: boolean };
}

function read<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function write(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // storage full or unavailable; preferences are best-effort
  }
}

export const prefs = {
  get vendor(): Vendor {
    return read<Vendor>(KEYS.vendor, "tcgplayer");
  },
  set vendor(value: Vendor) {
    write(KEYS.vendor, value);
  },
  get floor(): number {
    return read<number>(KEYS.floor, 0);
  },
  set floor(value: number) {
    write(KEYS.floor, value);
  },
  /** Arena-only boosters have no physical product or sealed price; hidden by default. */
  get showArena(): boolean {
    return read<boolean>(KEYS.showArena, false);
  },
  set showArena(value: boolean) {
    write(KEYS.showArena, value);
  },
  get selection(): Selection | null {
    return read<Selection | null>(KEYS.selection, null);
  },
  set selection(value: Selection | null) {
    write(KEYS.selection, value);
  },
  get history(): HistoryEntry[] {
    return read<HistoryEntry[]>(KEYS.history, []);
  },
  pushHistory(entry: HistoryEntry): void {
    write(KEYS.history, [entry, ...prefs.history].slice(0, 200));
  },
  clearHistory(): void {
    write(KEYS.history, []);
  },
};
