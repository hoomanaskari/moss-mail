import { useEffect } from "react";
import { Info, Leaf } from "lucide-react";
import { cx } from "../../lib/cx";
import { Icon, Toggle } from "../../design-system/atoms";
import { ListCard, ListRow, SectionHeader } from "../../design-system/molecules";
import { useSummarySettings } from "../../state/summary";
import shared from "./settings.module.css";
import styles from "./SummariesSettings.module.css";

/**
 * Settings → AI → Summaries: the automatic one-liners. "Always summarise" rules aren't listed
 * here: you set one from an email (S, then Always) and stop it from a summary it wrote.
 */
export function SummariesSettings() {
  const { automatic, problem, load, setAutomatic } = useSummarySettings();

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className={shared.group}>
      <SectionHeader label="Summaries" />
      <ListCard aria-label="Summaries">
        <ListRow className={styles.row}>
          <span className={cx(styles.tile, styles.ai)}>
            <Icon icon={Leaf} size={16} tone="ai" />
          </span>
          <span className={shared.text}>
            <span className={shared.name}>Auto summaries</span>
            <span className={shared.sub}>One line on conversations of 2+ emails that are in Important, or that you’ve replied in.</span>
          </span>
          <Toggle on={automatic} label="Auto summaries" onChange={(on) => void setAutomatic(on)} />
        </ListRow>
      </ListCard>
      {problem && <p className={shared.small}>{problem}</p>}
      <p className={styles.note}>
        <Icon icon={Info} size={14} tone="ink-3" />
        Rules live on the email: press S to set one, Stop rule to end it.
      </p>
    </div>
  );
}
