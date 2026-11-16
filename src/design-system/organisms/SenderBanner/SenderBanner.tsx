import { Ban, Undo2, UserRoundCheck, UserRoundX } from "lucide-react";
import { Button, Icon } from "../../atoms";
import { ShortcutHint } from "../../molecules";
import styles from "./SenderBanner.module.css";

export interface SenderBannerProps {
  /** `new`: someone you've never written to. `blocked`: mail opened from Blocked. */
  status: "new" | "blocked";
  email: string;
  /** Their domain, when it can be blocked or accepted as a whole. Left out for public ones like gmail.com. */
  domain?: string;
  /** The account's name, for where blocked mail goes. */
  account: string;
  /** Blocked for the address it came to (e.g. a shop address that leaked), not its sender. */
  blockedAddress?: string;
  onBlock: () => void;
  onBlockDomain: () => void;
  onAccept: () => void;
  onAcceptDomain: () => void;
  onUnblock: () => void;
}

/** On the email itself: "Block this sender?" for a new sender, or Unblock on blocked mail. */
export function SenderBanner(props: SenderBannerProps) {
  const { status, email, domain, account, blockedAddress, onBlock, onBlockDomain, onAccept, onAcceptDomain, onUnblock } = props;
  const byAddress = status === "blocked" && blockedAddress;
  const title = status === "new" ? "Block this sender?" : byAddress ? `You blocked mail to ${blockedAddress}` : "You blocked this sender";
  return (
    <section className={styles.banner} aria-label={title}>
      <div className={styles.top}>
        <Icon icon={status === "new" ? UserRoundX : Ban} size={18} tone="ink-2" />
        <div className={styles.text}>
          <span className={styles.title}>{title}</span>
          <span className={styles.why}>
            {status === "new"
              ? `From ${email}. You’ve never written to them.`
              : byAddress
                ? `Everything sent to it goes to ${account}’s Blocked folder, whoever it’s from.`
                : `Their mail goes to ${account}’s Blocked folder and never reaches your inbox.`}
          </span>
        </div>
      </div>
      <div className={styles.actions}>
        {status === "new" ? (
          <>
            <Button variant="secondary" icon={Ban} shortcut="B" className={styles.action} onClick={onBlock}>
              Block
            </Button>
            <Button variant="secondary" icon={UserRoundCheck} shortcut="A" className={styles.action} onClick={onAccept}>
              Accept
            </Button>
            {domain && (
              <div role="group" aria-label={`Everyone at ${domain}`} className={styles.domain}>
                <span>Everyone at {domain}</span>
                <ShortcutHint keys="⇧B" label="Block" tone="ink-2" gap={6} onClick={onBlockDomain} />
                <ShortcutHint keys="⇧A" label="Accept" tone="ink-2" gap={6} onClick={onAcceptDomain} />
              </div>
            )}
          </>
        ) : (
          <Button variant="secondary" icon={Undo2} shortcut="U" className={styles.action} onClick={onUnblock}>
            Unblock
          </Button>
        )}
      </div>
    </section>
  );
}
