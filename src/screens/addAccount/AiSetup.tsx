import { Cpu, ShieldCheck } from "lucide-react";
import { providerById } from "../../data/providers";
import { Button, Icon, Radio } from "../../design-system/atoms";
import { ListCard, ListRow, ProviderKind, ScreenHeading, SectionHeader } from "../../design-system/molecules";
import { FlowLayout } from "../../design-system/templates";
import type { AiPlan, AiStatus } from "../../services/ai";
import { aiChoices, useAddAccount, type Connected } from "../../state/addAccount";
import shared from "../settings/settings.module.css";
import styles from "./AddAccount.module.css";

/** "About 1.1 GB" → "1.1 GB". */
const bare = (size: string) => size.replace("About ", "");

/** Which model does each job, and how big it is. */
const sortAndSearch = (s: AiStatus) =>
  `d3-lite to sort (${bare(s.deciderSize)}) · EmbeddingGemma to search (${bare(s.embedderSize)})`;

function withWriter(s: AiStatus) {
  const writer = s.writers.find((w) => w.id === s.writer);
  return `${sortAndSearch(s)} · ${writer ? `${writer.name} to write (${bare(writer.size)})` : "Gemma to write"}`;
}

const copy: Record<AiPlan, { name: string; sub: string; models?: (status: AiStatus) => string }> = {
  recommended: {
    name: "Recommended",
    sub: "Everything: sorting, search by meaning, a one-line summary of what each email asks, and reply drafts.",
    models: withWriter,
  },
  essentials: {
    name: "Essentials only",
    sub: "Sorting and search by meaning. No summaries or reply drafts.",
    models: sortAndSearch,
  },
  skip: {
    name: "Skip for now",
    sub: "Simple rules sort your mail and search finds exact words only. Add AI later in Settings → AI.",
  },
};

/** First run, after the first account: what AI to get on this Mac. Recommended is picked for the real Mac. */
export function AiSetup({ account, status }: { account: Connected; status: AiStatus }) {
  const f = useAddAccount();
  const provider = providerById(f.provider!);
  const size = (plan: AiPlan) => (plan === "skip" ? "Nothing to download" : status.plans.find((p) => p.plan === plan)?.size);

  return (
    <FlowLayout gap={24}>
      <ProviderKind letter={provider.letter} name={provider.name} extra={account.address} />
      <ScreenHeading
        title="AI on this Mac"
        sub="Moss can run AI right on this Mac. Pick how much to download now. You can change it any time in Settings → AI."
      />
      <div className={shared.group}>
        <SectionHeader
          label="What to get"
          right={
            <span className={styles.machine}>
              <Icon icon={Cpu} size={14} tone="ink-3" />
              <span className={shared.small}>This Mac: {status.machine}</span>
            </span>
          }
        />
        <ListCard role="radiogroup" aria-label="What to get">
          {aiChoices.map((plan, i) => {
            const c = copy[plan];
            return (
              <ListRow
                as="label"
                key={plan}
                selected={i === f.aiChoice}
                className={styles.aiChoice}
              >
                <Radio name="ai" checked={i === f.aiChoice} onChange={() => useAddAccount.setState({ aiChoice: i })} />
                <span className={styles.aiText}>
                  <span className={shared.name}>{c.name}</span>
                  <span className={styles.aiSub}>{c.sub}</span>
                  {c.models && <span className={shared.small}>{c.models(status)}</span>}
                </span>
                <span className={styles.aiSize}>{size(plan)}</span>
              </ListRow>
            );
          })}
        </ListCard>
      </div>
      <div className={styles.aiNote}>
        <Icon icon={ShieldCheck} size={16} tone="ai" />
        <span className={styles.aiText}>
          <span className={styles.aiNoteLine}>Runs only on this Mac. Your mail never leaves it.</span>
          <span className={styles.aiNoteSub}>
            Downloads in the background, smallest first, and each feature turns on as its model arrives. Pauses when the
            battery is below 20% or you’re on a phone hotspot.
          </span>
        </span>
      </div>
      <div className={styles.actions}>
        <Button variant="primary" size="lg" shortcut="↵" onClick={() => void f.chooseAi()}>
          Download
        </Button>
        <Button variant="outline" size="lg" shortcut="S" onClick={() => void f.chooseAi("skip")}>
          Skip
        </Button>
      </div>
    </FlowLayout>
  );
}
