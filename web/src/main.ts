import { registerSW } from "virtual:pwa-register";
import { api, FLOORS, VENDORS, type BoosterListing, type PricingOptions, type Status, type Vendor } from "./api";
import { prefs } from "./state";
import { clear, h, mount, relativeTime } from "./ui";
import { rankingsView } from "./views/rankings";
import { simulatorView } from "./views/simulator";
import "./style.css";

// A new build waits for the user to accept it instead of swapping files
// under the open page; long-lived PWA windows check for one periodically.
const UPDATE_CHECK_MS = 60 * 60 * 1000;
const updateSW = registerSW({
  immediate: true,
  onNeedRefresh: () => showUpdateToast(),
  onRegisteredSW: (_url, registration) => {
    if (!registration) return;
    const check = () => void registration.update().catch(() => {});
    setInterval(check, UPDATE_CHECK_MS);
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") check();
    });
  },
});

function dismissToast(): void {
  document.querySelector(".toast")?.remove();
}

function showUpdateToast(): void {
  if (document.querySelector(".toast")) return;
  const toast = h(
    "div",
    { class: "toast", role: "status" },
    h("span", null, "A new version of packrat is available."),
    h("button", { type: "button", class: "toast-action", onClick: () => void updateSW() }, "Reload"),
    h("button", { type: "button", class: "toast-dismiss", ariaLabel: "Dismiss", onClick: dismissToast }, "×"),
  );
  document.body.append(toast);
}

const app = document.getElementById("app")!;
const pricingListeners = new Set<() => void>();
let pricing: PricingOptions = { vendor: prefs.vendor, floor: prefs.floor };

function notifyPricing(): void {
  for (const listener of pricingListeners) listener();
}
let listings: BoosterListing[] = [];
let status: Status | null = null;

const vendorSelect = h(
  "select",
  {
    class: "vendor-select",
    title: "Price source",
    onChange: () => {
      pricing = { ...pricing, vendor: vendorSelect.value as Vendor };
      prefs.vendor = pricing.vendor;
      notifyPricing();
    },
  },
  ...VENDORS.map((v) => h("option", { value: v.id }, v.label)),
);
vendorSelect.value = pricing.vendor;

const floorSelect = h(
  "select",
  {
    class: "floor-select",
    title: "Cards priced below this count as $0",
    onChange: () => {
      pricing = { ...pricing, floor: Number(floorSelect.value) };
      prefs.floor = pricing.floor;
      notifyPricing();
    },
  },
  ...FLOORS.map((f) => h("option", { value: String(f) }, f === 0 ? "Count all cards" : `Ignore cards under $${f}`)),
);
floorSelect.value = String(pricing.floor);

const statusEl = h("span", { class: "status" });
const nav = h(
  "nav",
  null,
  h("a", { href: "#/", class: "brand" }, h("span", { class: "brand-mark" }, "📦"), "packrat"),
  h("a", { href: "#/", dataset: { route: "sim" } }, "Simulator"),
  h("a", { href: "#/ev", dataset: { route: "ev" } }, "EV rankings"),
  h("span", { class: "spacer" }),
  statusEl,
  vendorSelect,
  floorSelect,
);
const main = h("main");
mount(app, h("header", { class: "topbar" }, nav), main);

const ctx = {
  get listings() {
    return listings;
  },
  pricing: () => pricing,
  onPricingChange: (cb: () => void) => {
    pricingListeners.add(cb);
  },
  openSimulator: (set_code: string, booster_code: string | null) => {
    const listing =
      listings.find((l) => l.set_code === set_code && l.booster_code === booster_code) ??
      listings.find((l) => l.set_code === set_code);
    if (listing) prefs.selection = { set_code: listing.set_code, booster_code: listing.booster_code };
    location.hash = "#/";
  },
};

function renderStatus(): void {
  clear(statusEl);
  if (!status) return;
  const parts: string[] = [];
  if (status.busy) parts.push(status.message);
  else if (status.error) parts.push(`Last refresh failed: ${status.error}`);
  else parts.push(`prices ${relativeTime(status.prices_updated_at)}`);
  mount(statusEl, 
    h("span", { class: `dot ${status.busy ? "busy" : status.error ? "error" : "ok"}` }),
    parts.join(" · "),
    status.mtgjson_version ? h("span", { class: "muted small" }, ` · MTGJSON ${status.mtgjson_version.split("+")[0]}`) : null,
  );
}

async function pollStatus(): Promise<void> {
  try {
    status = await api.status();
  } catch {
    status = null;
  }
  renderStatus();
  setTimeout(() => void pollStatus(), status?.busy ? 3000 : 60_000);
}

function route(): void {
  pricingListeners.clear();
  clear(main);
  const hash = location.hash || "#/";
  const current = hash.startsWith("#/ev") ? "ev" : "sim";
  for (const link of nav.querySelectorAll<HTMLAnchorElement>("a[data-route]")) {
    link.classList.toggle("active", link.dataset.route === current);
  }
  if (current === "ev") {
    mount(main, rankingsView(ctx));
  } else if (listings.length === 0) {
    mount(main, waitingView());
  } else {
    mount(main, simulatorView({ listings, pricing: ctx.pricing, onPricingChange: ctx.onPricingChange }));
  }
}

function waitingView(): HTMLElement {
  const el = h("section", { class: "waiting" });
  mount(el, 
    h("h2", null, status?.busy ? "Setting up the catalog" : "Catalog is not ready"),
    h("p", { class: "muted" }, status?.message ?? "Waiting for the server…"),
    h(
      "p",
      { class: "muted small" },
      "The first start downloads MTGJSON (about 130 MB) and prices; this takes a few minutes and only happens once.",
    ),
  );
  return el;
}

async function loadListings(): Promise<void> {
  try {
    const result = await api.products();
    if (result.length > 0) {
      listings = result;
      if (!location.hash.startsWith("#/ev")) route();
      return;
    }
  } catch {
    // retry below
  }
  setTimeout(() => void loadListings(), 4000);
}

window.addEventListener("hashchange", route);
route();
void pollStatus();
void loadListings();
