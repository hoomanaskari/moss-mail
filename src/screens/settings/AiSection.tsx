import { useEffect, useState } from "react";
import { Check, CircleCheck, Cpu, Download, ListFilter, Search, TriangleAlert } from "lucide-react";
import { amount, size } from "../../lib/bytes";
import { cx } from "../../lib/cx";
import { Button, Icon, Pill, ProgressBar, Radio } from "../../design-system/atoms";
import { ListCard, ListRow, ScreenHeading, SectionHeader } from "../../design-system/molecules";
import type { AiModelDownload, AiStatus, AiWriter } from "../../services/ai";
import { useAiSettings, watchAi } from "../../state/aiSettings";
import { useScreenKeys } from "../../state/useScreenKeys";
import shared from "./settings.module.css";
import styles from "./AiSection.module.css";
import { McpSection } from "./McpSection";
import { SummariesSettings } from "./SummariesSettings";

/** The teal "Ready" mark on a model that's on this Mac. */
function Ready() {
  return (
    <span className={styles.found}>
      <Icon icon={Check} size={14} tone="ai" />
      Ready
    </span>
  );
}

interface DeleteProps {
  id: string;
  name: string;
  /** Asking "Yes, delete?" now. */
  asking: boolean;
  /** Shows ⌘⌫ on it: the writing model under the cursor. */
  shortcut?: boolean;
  onDelete(id: string): void;
}

/** Deletes a model on this Mac. The first click asks; the second deletes. */
function DeleteButton({ id, name, asking, shortcut, onDelete }: DeleteProps) {
  return (
    <Button
      variant="secondary"
      shortcut={shortcut ? "⌘⌫" : undefined}
      aria-label={asking ? `Yes, delete ${name}` : `Delete ${name}`}
      onClick={() => onDelete(id)}
    >
      {asking ? "Yes, delete" : "Delete"}
    </Button>
  );
}

/** A sorting or search model's own line, while it isn't simply in. */
function sharedLine(d: AiModelDownload) {
  switch (d.state) {
    case "downloading":
      return `Downloading… ${size(d.done)}`;
    case "queued":
      return "Waiting to download";
    case "failed":
      return d.message;
    default:
      return undefined;
  }
}

/** The two models that sort and search: what each does, its size, and whether it's in. */
function SortingAndSearch({ status, asking, onDelete }: { status: AiStatus; asking?: string; onDelete(id: string): void }) {
  const { downloadModels, stopModel } = useAiSettings();
  const models = [
    { id: "decider", name: "d3-lite", sub: "Sorts your mail and spots what needs your reply.", icon: ListFilter, size: status.deciderSize, download: status.deciderDownload },
    { id: "embedder", name: "EmbeddingGemma", sub: "Finds emails by meaning, not just by exact words.", icon: Search, size: status.embedderSize, download: status.embedderDownload },
  ];
  /** Smallest first, as a full download gets them. */
  const both = ["embedder", "decider"];
  const getting = models.some((m) => m.download.state === "downloading" || m.download.state === "queued");
  const missing = models.some((m) => m.download.state !== "ready");
  return (
    <div className={shared.group}>
      <SectionHeader label="Sorting and search" />
      <ListCard as="ul" aria-label="Sorting and search">
        {models.map((m) => {
          const line = sharedLine(m.download);
          return (
            <ListRow as="li" key={m.id} className={styles.provider}>
              <span className={styles.tile}>
                <Icon icon={m.icon} size={16} tone="ink-2" />
              </span>
              <span className={shared.text}>
                <span className={shared.name}>{m.name}</span>
                <span className={cx(shared.sub, styles.clip)}>{m.sub}</span>
              </span>
              {line && <span className={cx(shared.small, m.download.state === "failed" && styles.failed)}>{line}</span>}
              {m.download.state === "ready" && <Ready />}
              <span className={styles.size}>{m.size}</span>
              <span className={styles.action}>
                {status.downloaded.includes(m.id) && <DeleteButton id={m.id} name={m.name} asking={asking === m.id} onDelete={onDelete} />}
              </span>
            </ListRow>
          );
        })}
        {missing && (
          <li className={styles.download}>
            <Icon icon={models.some((m) => m.download.state === "failed") ? TriangleAlert : Download} size={16} tone="ink-2" />
            <span className={styles.downloadText}>Sorting and search download on their own, apart from the writing model.</span>
            {getting ? (
              <Button variant="secondary" aria-label="Pause sorting and search" onClick={() => both.forEach((id) => void stopModel(id))}>
                Pause
              </Button>
            ) : (
              <Button variant="secondary" aria-label="Download sorting and search" onClick={() => void downloadModels(both)}>
                Download
              </Button>
            )}
          </li>
        )}
      </ListCard>
    </div>
  );
}

/** A writing model's line: how much of it is in while it downloads, else what it's for. */
function modelLine(w: AiWriter) {
  const d = w.download;
  switch (d.state) {
    case "queued":
      return "Waiting to download";
    case "downloading":
      return `Downloading… ${amount(d.done, w.bytes)} · ${Math.floor((d.done / w.bytes) * 100)}%`;
    case "failed":
      return d.done ? `${d.message} ${size(d.done)} is kept.` : d.message;
    case "missing":
      return d.done ? `Paused at ${amount(d.done, w.bytes)}` : w.sub;
    case "ready":
      return w.sub;
  }
}

/** What a writing model's download button does now, if it has one: Download, Resume, Pause or Try again. */
function downloadAction(w: AiWriter): [label: string, act: "download" | "stop"] | undefined {
  const d = w.download;
  if (w.tooBig || d.state === "ready") return undefined;
  if (d.state === "failed") return ["Try again", "download"];
  if (d.state === "missing") return [d.done ? "Resume" : "Download", "download"];
  return ["Pause", "stop"];
}

/** A download of `w` that can be cancelled: running, or with some of it on this Mac. */
function cancellable(w: AiWriter) {
  const d = w.download;
  return d.state === "downloading" || (d.state !== "ready" && d.done > 0);
}

/** Settings → AI: the models that run on this Mac, and Summaries. */
export function AiSection() {
  const { status, problem, load, chooseModel, downloadModels, stopModel, removeModel } = useAiSettings();
  const writers = status?.writers ?? [];
  const chosen = Math.max(0, writers.findIndex((m) => m.id === status?.writer));
  const [moved, setMoved] = useState<number>();
  const cursor = moved ?? chosen;
  /** The model a Delete is asking about. */
  const [asking, setAsking] = useState<string>();

  const setCursor = (i: number) => {
    setMoved(i);
    setAsking(undefined);
  };

  /** Asks first; deletes when asked again. */
  const remove = (id: string) => {
    if (asking !== id) return setAsking(id);
    setAsking(undefined);
    void removeModel(id);
  };

  /** Downloads, resumes, pauses or tries `w` again. */
  const act = (w: AiWriter) => {
    const action = downloadAction(w);
    if (action?.[1] === "download") void downloadModels([w.id]);
    else if (action) void stopModel(w.id);
  };

  /** Chooses `w`: it writes now if it's in, or once it is. One downloading already is left be. */
  const pick = (w: AiWriter, i: number) => {
    setCursor(i);
    if (w.download.state === "ready") void chooseModel(w.id);
    else if (downloadAction(w)?.[1] === "download") void downloadModels([w.id]);
  };

  /** Stops the download of `w` and deletes what's in of it. */
  const cancel = (w: AiWriter) => {
    setAsking(undefined);
    void removeModel(w.id);
  };

  useEffect(() => {
    void load();
    return watchAi();
  }, [load]);

  useScreenKeys((k, e) => {
    if (k.key === "escape" && asking) {
      setAsking(undefined);
      e.stopImmediatePropagation();
      return true;
    }
    if (!status) return;
    const under = writers[cursor];
    if (k.mod && k.key === "backspace") {
      if (cancellable(under)) cancel(under);
      else if (status.downloaded.includes(under.id)) remove(under.id);
      return true;
    }
    if (k.typing || k.mod || k.alt) return;
    if (k.key === "arrowdown" || k.key === "arrowup")
      return setCursor(Math.min(writers.length - 1, Math.max(0, cursor + (k.key === "arrowdown" ? 1 : -1)))), true;
    if (k.key === " ") {
      if (!under.tooBig) pick(under, cursor);
      return true;
    }
    if (k.key === "d") return act(under), true;
  });

  const tookOver = writers.find((w) => w.id === status?.tookOver && w.id === status.writer);

  return (
    <div className={shared.page}>
      <ScreenHeading title="AI" sub="Moss runs its AI on this Mac. Your email never leaves it." />

      {status && <SortingAndSearch status={status} asking={asking} onDelete={remove} />}

      {status && (
        <div className={shared.group}>
          <SectionHeader
            label="Writing model"
            right={
              <span className={styles.machine}>
                <Icon icon={Cpu} size={14} tone="ink-3" />
                <span className={shared.small}>This Mac: {status.machine}</span>
              </span>
            }
          />
          <ListCard as="ul" role="radiogroup" aria-label="Writing model">
            {writers.map((w, i) => {
              const d = w.download;
              const action = downloadAction(w);
              const showsProgress = d.state === "downloading" || (d.state === "missing" && d.done > 0);
              return (
                <ListRow as="li" key={w.id} selected={i === cursor} className={cx(styles.model, w.tooBig && styles.tooBig)}>
                  <label className={styles.choice}>
                    <Radio
                      name="model"
                      aria-label={w.name}
                      checked={status.writer === w.id && d.state === "ready"}
                      disabled={w.tooBig}
                      onChange={() => pick(w, i)}
                    />
                    <span className={cx(shared.text, styles.modelText)}>
                      <span className={styles.nameRow}>
                        <span className={shared.name}>{w.name}</span>
                        {w.recommended && <Pill variant="label">Recommended</Pill>}
                      </span>
                      <span className={cx(shared.sub, styles.clip, d.state === "failed" && styles.failed)}>
                        {d.state === "failed" && (
                          <span className={styles.warn}>
                            <Icon icon={TriangleAlert} size={13} tone="due" />
                          </span>
                        )}
                        {modelLine(w)}
                      </span>
                      {showsProgress && (
                        <span className={styles.bar}>
                          <ProgressBar value={d.done / w.bytes} label={`${w.name} download`} />
                        </span>
                      )}
                    </span>
                    {w.tooBig && <span className={shared.small}>Needs more memory</span>}
                    {d.state === "ready" && <Ready />}
                    {d.state !== "downloading" && <span className={styles.size}>{w.size}</span>}
                  </label>
                  <span className={styles.action}>
                    {action && (
                      <Button variant="secondary" shortcut={i === cursor ? "D" : undefined} aria-label={`${action[0]} ${w.name}`} onClick={() => act(w)}>
                        {action[0]}
                      </Button>
                    )}
                    {cancellable(w) && (
                      <Button variant="secondary" shortcut={i === cursor ? "⌘⌫" : undefined} aria-label={`Cancel ${w.name}`} onClick={() => cancel(w)}>
                        Cancel
                      </Button>
                    )}
                    {!action && status.downloaded.includes(w.id) && (
                      <DeleteButton id={w.id} name={w.name} asking={asking === w.id} shortcut={i === cursor} onDelete={remove} />
                    )}
                  </span>
                </ListRow>
              );
            })}
          </ListCard>
          {tookOver && (
            <p className={cx(shared.small, styles.note)} role="status">
              <Icon icon={CircleCheck} size={14} tone="ai" />
              {tookOver.name} is ready and now writes for you.
            </p>
          )}
          {problem && <p className={cx(shared.small, styles.problem)}>{problem}</p>}
        </div>
      )}

      {status && <SummariesSettings />}

      <McpSection />
    </div>
  );
}
