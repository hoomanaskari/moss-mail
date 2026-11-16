import { useEffect, useState } from "react";
import { Button, PageTitle } from "../design-system/atoms";
import { ListCard, ListRow, SectionHeader, ShortcutHint } from "../design-system/molecules";
import { ShortcutBar, TopBar } from "../design-system/organisms";
import { AppFrame } from "../design-system/templates";
import type { SenderRule } from "../data/types";
import { listTime } from "../lib/time";
import { useSenderRules } from "../state/senderRules";
import { accountFor, useStore } from "../state/store";
import { useScreenKeys } from "../state/useScreenKeys";
import shared from "./settings/settings.module.css";
import styles from "./SendersScreen.module.css";

const hints = [
  { keys: "↑ ↓", label: "Move" },
  { keys: "⌫", label: "Remove" },
  { keys: "Esc", label: "Back" },
];

/** Who Moss keeps out and who it lets through, with a way to undo either. Reached from ⌘K. */
export function SendersScreen() {
  const { rules, problem, load, forget } = useSenderRules();
  const openBlocked = useStore((s) => s.openBlocked);
  const leave = useStore((s) => s.leave);
  const [cursor, setCursor] = useState(0);

  useEffect(() => {
    void load();
  }, [load]);

  const blocked = rules?.filter((r) => r.blocked) ?? [];
  const accepted = rules?.filter((r) => !r.blocked) ?? [];
  const ordered = [...blocked, ...accepted];
  // After a removal the cursor stays on the last rule left.
  const at = Math.min(cursor, ordered.length - 1);

  useScreenKeys((k) => {
    if (k.key === "escape") return leave(), true;
    if (k.typing || k.mod || k.alt) return;
    if (k.key === "arrowdown" || k.key === "arrowup")
      return setCursor(Math.min(ordered.length - 1, Math.max(0, at + (k.key === "arrowdown" ? 1 : -1)))), true;
    if (k.key === "backspace" && ordered[at]) return void forget(ordered[at]), true;
  });

  const now = Date.now();

  const list = (label: string, items: SenderRule[], empty: string) => (
    <ListCard as="ul" aria-label={label}>
      {items.length === 0 && (
        <ListRow as="li" className={styles.empty}>
          {empty}
        </ListRow>
      )}
      {items.map((r) => {
        const selected = ordered[at] === r;
        return (
          <ListRow as="li" key={`${r.accountId}:${r.sender}`} selected={selected} aria-current={selected || undefined} className={styles.row}>
            <span className={shared.text}>
              <span className={shared.name}>{r.wholeDomain ? `Everyone at ${r.sender}` : r.sender}</span>
              <span className={shared.sub}>
                {accountFor(r.accountId).name} · {listTime(r.createdAt, now)}
              </span>
            </span>
            <Button variant="secondary" onClick={() => void forget(r)}>
              {r.blocked ? "Unblock" : "Remove"}
            </Button>
          </ListRow>
        );
      })}
    </ListCard>
  );

  return (
    <AppFrame
      topBar={<TopBar title={<PageTitle>Senders</PageTitle>} right={<ShortcutHint keys="Esc" label="Back" tone="ink-2" onClick={leave} />} />}
      footer={<ShortcutBar hints={hints} />}
    >
      <div className={styles.scroll}>
        <div className={`${shared.page} ${styles.column}`}>
          <p className={styles.sub}>Who Moss keeps out, and who it lets through without asking. Remove someone and Moss asks about them again.</p>
          {problem && <p className={shared.small}>{problem}</p>}
          {rules && (
            <>
              <div className={shared.group}>
                <SectionHeader
                  label="Blocked"
                  right={<ShortcutHint keys="⌘⇧B" label="Blocked mail" tone="ink-2" labelFirst onClick={openBlocked} />}
                />
                {list("Blocked", blocked, "No one is blocked. On a new sender’s email, B blocks them.")}
              </div>

              <div className={shared.group}>
                <SectionHeader label="Accepted" />
                {list("Accepted", accepted, "No one accepted yet. A accepts a new sender, ⇧A everyone at their domain.")}
              </div>
            </>
          )}
        </div>
      </div>
    </AppFrame>
  );
}
