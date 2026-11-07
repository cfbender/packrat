type Child = Node | string | number | null | undefined | false | Child[];

type Props<K extends keyof HTMLElementTagNameMap> = Partial<
  Omit<HTMLElementTagNameMap[K], "style" | "children" | "dataset">
> & {
  class?: string;
  style?: Partial<CSSStyleDeclaration>;
  dataset?: Record<string, string>;
  onClick?: (event: MouseEvent) => void;
  onInput?: (event: Event) => void;
  onChange?: (event: Event) => void;
  onKeydown?: (event: KeyboardEvent) => void;
};

function append(parent: Node, child: Child): void {
  if (child === null || child === undefined || child === false) return;
  if (Array.isArray(child)) {
    for (const c of child) append(parent, c);
    return;
  }
  parent.appendChild(child instanceof Node ? child : document.createTextNode(String(child)));
}

/** Tiny element builder: h("div", { class: "x" }, child, ...). */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Props<K> | null = null,
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (props) {
    const { class: className, style, dataset, onClick, onInput, onChange, onKeydown, ...rest } = props;
    if (className) el.className = className;
    if (style) Object.assign(el.style, style);
    if (dataset) Object.assign(el.dataset, dataset);
    if (onClick) el.addEventListener("click", onClick as EventListener);
    if (onInput) el.addEventListener("input", onInput);
    if (onChange) el.addEventListener("change", onChange);
    if (onKeydown) el.addEventListener("keydown", onKeydown as EventListener);
    Object.assign(el, rest);
  }
  for (const child of children) append(el, child);
  return el;
}

export function clear(el: Element): void {
  el.replaceChildren();
}

/** Appends children, skipping null/false like `h` does. */
export function mount(parent: Element, ...children: Child[]): void {
  for (const child of children) append(parent, child);
}

const usd = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" });

export function money(dollars: number | null | undefined): string {
  if (dollars === null || dollars === undefined || Number.isNaN(dollars)) return "—";
  return usd.format(dollars);
}

export function cents(value: number | null | undefined): string {
  return money(value === null || value === undefined ? null : value / 100);
}

export function percent(ratio: number | null | undefined, digits = 0): string {
  if (ratio === null || ratio === undefined || Number.isNaN(ratio)) return "—";
  return `${(ratio * 100).toFixed(digits)}%`;
}

/** Accepts an ISO string or unix seconds (the server stores the latter). */
export function relativeTime(stamp: string | null | undefined): string {
  if (!stamp) return "never";
  const then = /^\d+$/.test(stamp) ? Number(stamp) * 1000 : new Date(stamp).getTime();
  if (Number.isNaN(then)) return stamp;
  const minutes = Math.round((Date.now() - then) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
}

/** Botbox-style verdict tiers based on EV/price and break-even share. */
export function verdict(
  ratio: number | null,
  breakEven: number | null,
  sd: number,
  price: number | null,
): { label: string; tone: "good" | "ok" | "meh" | "bad" | "lottery" | "none"; blurb: string } {
  if (ratio === null || price === null) {
    return { tone: "none", label: "No price", blurb: "No sealed price is known for this product at the chosen vendor." };
  }
  if (breakEven !== null && breakEven <= 0.15 && sd >= 0.75 * price) {
    return {
      tone: "lottery",
      label: "Lottery ticket",
      blurb: "Most packs lose money; the average is propped up by rare high-value pulls.",
    };
  }
  if (ratio >= 1.15) return { tone: "good", label: "Positive EV", blurb: "On average, singles are worth more than the pack." };
  if (ratio >= 0.95) return { tone: "ok", label: "Near break-even", blurb: "Expected value roughly matches the price." };
  if (ratio >= 0.8) return { tone: "meh", label: "Fair", blurb: "A modest expected loss, typical for most sealed product." };
  if (ratio >= 0.6) return { tone: "bad", label: "Poor", blurb: "Expect to lose a noticeable share of the price." };
  return { tone: "bad", label: "Very poor", blurb: "Buy singles instead." };
}
