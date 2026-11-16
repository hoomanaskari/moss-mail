/** "38 MB", "6.4 GB", "1 GB": sizes as Moss shows them. */
export function size(n: number) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1).replace(/\.0$/, "")} GB`;
  return `${Math.round(n / 1e6)} MB`;
}

/** "2.4 of 6.1 GB", or "450 MB of 6.1 GB" when the units differ. */
export function amount(done: number, total: number) {
  const [d, t] = [size(done), size(total)];
  return d.slice(-2) === t.slice(-2) ? `${d.slice(0, -3)} of ${t}` : `${d} of ${t}`;
}
