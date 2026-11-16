import { Building2, Check, ExternalLink } from "lucide-react";
import { providerById } from "../../data/providers";
import { Button, Icon, SectionLabel } from "../../design-system/atoms";
import { ScreenHeading, Notice, ProviderKind } from "../../design-system/molecules";
import { SetupSteps, StatusCard } from "../../design-system/organisms";
import { FlowLayout } from "../../design-system/templates";
import { accountSetup } from "../../services/accountSetup";
import { useAddAccount } from "../../state/addAccount";
import styles from "./AddAccount.module.css";
import { AdvancedLink, Footnote, SetupProblem } from "./parts";
import { openLink } from "../../lib/links";

export const openInBrowser = (url: string) => openLink(url);
export const copyText = (text: string) => navigator.clipboard?.writeText(text);

/** Outlook, Hotmail and Microsoft 365: one click in the browser. */
export function OutlookSignIn() {
  const f = useAddAccount();
  const { letter, name } = providerById("outlook");
  const needsAdmin = f.error === "admin-approval";

  return (
    <FlowLayout gap={28}>
      <ProviderKind letter={letter} name={name} />
      {needsAdmin ? (
        <>
          <ScreenHeading
            title="Your organisation needs to approve Moss"
            sub="Microsoft says an IT admin at your work or school must allow Moss before you can sign in."
          />
          <SetupSteps
            label="How to fix it"
            steps={[
              {
                title: "Send your admin the approval link",
                sub: "They approve Moss once, for everyone at your work or school.",
                url: accountSetup().adminApprovalLink(),
                action: "Copy link",
                keys: "⌘C",
              },
              { title: "Try again once they’ve said yes" },
            ]}
            done={0}
            onOpen={(_, url) => void copyText(url)}
          />
          <div className={styles.actions}>
            <Button variant="primary" size="lg" shortcut="↵" disabled={f.busy} onClick={() => void f.connect()}>
              Try again
            </Button>
          </div>
          <Notice variant="note" icon={Building2}>
            Your admin may give you their own client ID instead. If so, choose “Use your own Microsoft client ID”.
          </Notice>
        </>
      ) : (
        <>
          <ScreenHeading
            title="Sign in with Microsoft"
            sub="Moss opens Microsoft’s sign-in page in your browser. Sign in there, then come back here."
          />
          <SetupProblem />
          <StatusCard
            kind="waiting"
            title="Waiting for you in the browser"
            sub="Finish signing in at login.microsoftonline.com. This screen moves on by itself."
            action={
              <Button variant="secondary" icon={ExternalLink} shortcut="⌘O" onClick={() => openInBrowser(accountSetup().signInLink())}>
                Open again
              </Button>
            }
          />
          <section className={styles.permissions}>
            <SectionLabel>Microsoft will ask you to let Moss</SectionLabel>
            <ul>
              <li>
                <Icon icon={Check} size={16} tone="ai" />
                Read, sort and send your email
              </li>
              <li>
                <Icon icon={Check} size={16} tone="ai" />
                Stay signed in, so you don’t have to do this again
              </li>
            </ul>
          </section>
          <Notice variant="note" icon={Building2}>
            Work or school account? Your IT admin may need to approve Moss once. Microsoft will tell you if so.
          </Notice>
        </>
      )}
      <AdvancedLink label="Use your own Microsoft client ID" onClick={f.toggleAdvanced} />
      <Footnote>Moss never sees your Microsoft password. It keeps a sign-in token in the macOS Keychain.</Footnote>
    </FlowLayout>
  );
}
