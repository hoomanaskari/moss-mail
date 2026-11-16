import { AtSign } from "lucide-react";
import { cx } from "../../../lib/cx";
import { Icon } from "../Icon/Icon";
import styles from "./ProviderTile.module.css";

export interface ProviderTileProps {
  /** The provider's letter; without one the tile shows an @ (Other email). */
  letter?: string;
  /** 36 on the provider list, 24 beside the provider's name. */
  size?: 36 | 24;
  /** On a raised row the tile drops to the surface colour so it still stands out. */
  raised?: boolean;
}

/** A neutral square naming an email provider. Provider logos are trademarks, so Moss uses a letter. */
export function ProviderTile({ letter, size = 36, raised }: ProviderTileProps) {
  return (
    <span className={cx(styles.tile, styles[`s${size}`], raised && styles.raised)} aria-hidden>
      {letter ?? <Icon icon={AtSign} size={size === 36 ? 18 : 14} tone="ink" />}
    </span>
  );
}
