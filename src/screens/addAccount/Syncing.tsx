import { useEffect } from "react";
import { Plus } from "lucide-react";
import { providerById } from "../../data/providers";
import { AccountBadge, Button } from "../../design-system/atoms";
import { ColorSwatches, ScreenHeading, ProviderKind, ShortcutHint } from "../../design-system/molecules";
import { StatusCard, SyncProgress } from "../../design-system/organisms";
import { FlowLayout } from "../../design-system/templates";
import { accountSetup } from "../../services/accountSetup";
import { useSync } from "../../services/live";
import { useAddAccount, type Connected } from "../../state/addAccount";
import { useAiSettings } from "../../state/aiSettings";
import styles from "./AddAccount.module.css";

const savedAs = (s: { provider?: string; advanced: boolean }) =>
  s.provider === "resend"
    ? "Connected. API key saved in the macOS Keychain"
    : s.provider === "outlook" || s.advanced
      ? "Signed in. Sign-in token saved in the macOS Keychain"
      : "Signed in. Password saved in the macOS Keychain";

/** The new account is in: mail is downloading, and you can pick its colour. */
export function Syncing({ account }: { account: Connected }) {
  const f = useAddAccount();
  const provider = providerById(f.provider!);
  const live = useSync((s) => s.progress[account.id]);
  const sync = live ?? accountSetup().syncProgress(account.address);
  const recentDone = sync.recent >= 1;
  // Whether AI on this Mac is still to be asked about, for the step after this one.
  useEffect(() => {
    void useAiSettings.getState().loadStatus();
  }, []);

  return (
    <FlowLayout gap={24}>
      <ProviderKind letter={provider.letter} name={provider.name} extra={account.address} />
      <ScreenHeading
        title={`${account.name} is connected`}
        sub="Your newest mail comes first. Older mail keeps downloading in the background."
      />
      <StatusCard
        kind="ready"
        title="You can start using Moss now."
        sub="Downloading carries on, even if you close this window."
        action={
          <Button variant="primary" size="lg" shortcut="↵" onClick={f.finish}>
            Go to inbox
          </Button>
        }
      />
      <SyncProgress
        done={sync.done}
        total={sync.total}
        items={[
          { label: savedAs(f), state: "done" },
          { label: f.provider === "gmail" ? "Folders and labels" : "Folders", state: "done", status: `${sync.folders} found` },
          { label: "Recent mail, last 30 days", state: recentDone ? "done" : "now", status: `${Math.round(sync.recent * 100)}%` },
          {
            label: `Older mail, back to ${sync.oldestYear}`,
            state: recentDone ? "now" : "next",
            status: recentDone ? "In the background" : "Next, in the background",
          },
        ]}
      />
      <div className={styles.colour}>
        <span className={styles.colourLabel}>Account colour</span>
        <AccountBadge letter={account.letter} color={account.color} size={28} />
        <ColorSwatches
          label="Account colour"
          value={account.color}
          onChange={(color) => useAddAccount.setState({ connected: { ...account, color } })}
        />
        <span className={styles.spacer} />
        <ShortcutHint keys="← →" label="Change" />
      </div>
      <div className={styles.actions}>
        <Button variant="outline" size="lg" icon={Plus} shortcut="A" onClick={f.addAnother}>
          Add another account
        </Button>
      </div>
    </FlowLayout>
  );
}
