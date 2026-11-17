import { join } from "node:path";

/**
 * The two Moss apps. The installed one is made only by `pnpm release`, from tauri.conf.json as
 * it is. Moss Dev is what `pnpm app` and `pnpm tauri dev` build, with tauri.dev.conf.json laid
 * over it: its own name, identifier, data, Keychain items and icon, and no updater.
 */
export const production = { identifier: "app.moss.mail", productName: "Moss" } as const;
export const development = { identifier: "app.moss.mail.dev", productName: "Moss Dev" } as const;

/** Where the installed Moss looks for new versions. */
export const updateFeed = "https://github.com/hoomanaskari/moss-mail/releases/latest/download/latest.json";

/** The overlay that makes Moss Dev, in the checkout at `root`. */
export const devOverlay = (root: string) => join(root, "src-tauri", "tauri.dev.conf.json");

/**
 * The arguments for Tauri's CLI when `pnpm tauri <args>` is run in the checkout at `root`:
 * `tauri dev` always runs Moss Dev. Everything else goes through as it is, `tauri build` too:
 * `pnpm app` names the overlay itself, and `pnpm release` and `pnpm check` must build the
 * installed Moss.
 */
export function tauriArgs(args: string[], root: string): string[] {
  return args[0] === "dev" ? ["dev", "--config", devOverlay(root), ...args.slice(1)] : args;
}

interface TauriConfig {
  identifier?: unknown;
  productName?: unknown;
  app?: { windows?: { title?: unknown }[] };
  plugins?: { updater?: { endpoints?: unknown } };
}

/** Stops a release whose config isn't the installed Moss's: the wrong one would replace it, or never update. */
export function assertProduction(config: TauriConfig): void {
  const problems = [
    config.identifier !== production.identifier && `identifier is ${String(config.identifier)}`,
    config.productName !== production.productName && `product name is ${String(config.productName)}`,
    config.app?.windows?.[0]?.title !== production.productName && "the window isn't called Moss",
    JSON.stringify(config.plugins?.updater?.endpoints) !== JSON.stringify([updateFeed]) && "the update feed isn't Moss's",
  ].filter(Boolean);
  if (problems.length) throw new Error(`This is not the installed Moss's config: ${problems.join(", ")}`);
}
