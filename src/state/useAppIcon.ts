import { useEffect } from "react";
import { appIcon, setAppIcon } from "../lib/appIcon";
import { useStore } from "./store";

/** Keeps the Dock icon in step with Moss's theme choice. */
export function useAppIcon() {
  const icon = useStore((s) => appIcon(s.themeChoice));
  useEffect(() => {
    void setAppIcon(icon);
  }, [icon]);
}
