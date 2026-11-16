import { BackupSettings } from "./BackupSettings";
import { useEffect, useState } from "react";
import { size as bytes } from "../../lib/bytes";
import { cx } from "../../lib/cx";
import { Button, Pill, Radio, Toggle } from "../../design-system/atoms";
import { ListCard, ListRow, ScreenHeading, SectionHeader, SegmentedControl } from "../../design-system/molecules";
import type { Offline } from "../../services/mail";
import { useAiSettings } from "../../state/aiSettings";
import { useStorageSettings } from "../../state/storageSettings";
import { useScreenKeys } from "../../state/useScreenKeys";
import shared from "./settings.module.css";
import styles from "./StorageSection.module.css";

const offlineOptions: { value: Offline; name: string; sub: string; size: string }[] = [
  { value: "recent", name: "Recent only", sub: "Emails from the last 90 days. Older ones come from the server when you open them.", size: "Smallest" },
  { value: "text", name: "All email text", sub: "Every email’s text, compressed. Attachments stay on the server.", size: "About 5 KB an email" },
  { value: "everything", name: "Everything", sub: "Every email, with its attachments. Can use a lot of space.", size: "Can be many GB" },
];

const limits = [250e6, 500e6, 1e9, 2e9];

/** Settings → Storage: where the space goes, what's kept for offline, and the cache. */
export function StorageSection() {
  const { info, problem, load, setOffline, setCacheLimit, setAttachmentText, clearCache, rebuildIndex } = useStorageSettings();
  const removeModels = useAiSettings((s) => s.remove);
  const [moved, setCursor] = useState<number>();
  const chosen = Math.max(0, offlineOptions.findIndex((o) => o.value === info?.offline));
  const cursor = moved ?? chosen;

  useEffect(() => {
    void load();
  }, [load]);

  const remove = async () => {
    await removeModels();
    await load();
  };

  useScreenKeys((k) => {
    if (k.typing || k.mod || k.alt || !info) return;
    if (k.key === "arrowdown" || k.key === "arrowup")
      return setCursor(Math.min(offlineOptions.length - 1, Math.max(0, cursor + (k.key === "arrowdown" ? 1 : -1)))), true;
    if (k.key === " ") return void setOffline(offlineOptions[cursor].value), true;
    if (k.key === "c") return void clearCache(), true;
    if (k.key === "r" && !info.rebuilding) return void rebuildIndex(), true;
    if (k.key === "m" && info.models > 0) return void remove(), true;
  });

  if (!info) return <div className={shared.page} />;
  const { usage } = info;
  const parts = [
    {
      id: "index",
      name: "Index & summaries",
      sub: info.rebuilding ? "Rebuilding. Search finds less until it’s done." : "Finds any email by word or by meaning, with one-line summaries.",
      size: bytes(usage.index),
      action: (
        <Button variant="secondary" shortcut="R" disabled={info.rebuilding} onClick={() => void rebuildIndex()}>
          Rebuild
        </Button>
      ),
    },
    { id: "offline", name: "Offline email", sub: "The text of your emails, compressed. No attachments.", size: bytes(usage.offline) },
    {
      id: "cache",
      name: "Cache",
      sub: "Emails and files you opened lately. Clearing keeps everything else.",
      size: `${bytes(usage.cache)} of ${bytes(info.cacheLimit)}`,
      action: (
        <Button variant="secondary" shortcut="C" onClick={() => void clearCache()}>
          Clear
        </Button>
      ),
    },
    {
      id: "models",
      name: "AI models",
      sub: info.models ? "Sorting, search by meaning, summaries and drafts. Remove turns these off." : "Not on this Mac. Settings → AI has them.",
      size: bytes(info.models),
      action: info.models > 0 && (
        <Button variant="secondary" shortcut="M" onClick={() => void remove()}>
          Remove
        </Button>
      ),
    },
  ];
  const total = usage.index + usage.offline + usage.cache + info.models;

  return (
    <div className={shared.page}>
      <ScreenHeading
        title="Storage"
        sub="Moss keeps a small index of all your mail, so it can find anything fast. Full emails and files come from the server when you open them."
      />

      <div className={shared.group}>
        <SectionHeader label="Space used" right={<span className={shared.small}>{bytes(total)} on this Mac</span>} />
        <ListCard>
          <div className={styles.bar} role="img" aria-label={parts.map((p) => `${p.name} ${p.size}`).join(", ")}>
            {parts.map((p) => (
              <span key={p.id} className={cx(styles.part, styles[p.id])} />
            ))}
          </div>
          {parts.map((p) => (
            <ListRow key={p.id} className={styles.row}>
              <span className={cx(styles.swatch, styles[p.id])} />
              <span className={shared.text}>
                <span className={shared.name}>{p.name}</span>
                <span className={cx(shared.sub, styles.wrap)}>{p.sub}</span>
              </span>
              <span className={styles.size}>{p.size}</span>
              <span className={styles.action}>{p.action}</span>
            </ListRow>
          ))}
        </ListCard>
      </div>

      <BackupSettings />
      <div className={shared.group}>
        <SectionHeader label="Keep on this Mac for offline" />
        <ListCard role="radiogroup" aria-label="Keep on this Mac for offline">
          {offlineOptions.map((o, i) => (
            <ListRow as="label" key={o.value} selected={i === cursor} className={styles.option}>
              <Radio
                name="offline"
                checked={info.offline === o.value}
                onChange={() => {
                  setCursor(i);
                  void setOffline(o.value);
                }}
              />
              <span className={shared.text}>
                <span className={styles.nameRow}>
                  <span className={shared.name}>{o.name}</span>
                  {o.value === "text" && <Pill variant="label">Default</Pill>}
                </span>
                <span className={cx(shared.sub, styles.wrap)}>{o.sub}</span>
              </span>
              <span className={styles.size}>{o.size}</span>
            </ListRow>
          ))}
          <div className={styles.setting}>
            <span className={shared.text}>
              <span className={shared.name}>Search inside attachments</span>
              <span className={cx(shared.sub, styles.wrap)}>
                Reads the text in PDFs, scans and documents on this Mac, so search and Ask Moss find what’s in them. Only the text
                is kept.
              </span>
            </span>
            <Toggle label="Search inside attachments" on={info.attachmentText} onChange={(on) => void setAttachmentText(on)} />
          </div>
          <div className={styles.setting}>
            <span className={shared.text}>
              <span className={shared.name}>Cache limit</span>
              <span className={cx(shared.sub, styles.wrap)}>
                Opened emails and files, plus the next few in your list so they open at once. The oldest go first.
              </span>
            </span>
            <SegmentedControl
              label="Cache limit"
              size="sm"
              value={String(info.cacheLimit)}
              options={limits.map((l) => ({ value: String(l), label: bytes(l) }))}
              onChange={(v) => void setCacheLimit(Number(v))}
            />
          </div>
        </ListCard>
        {problem && <p className={cx(shared.small, styles.problem)}>{problem}</p>}
      </div>
    </div>
  );
}
