import { Button } from "../../atoms";
import { Stat } from "../../molecules";
import styles from "./InboxZeroHero.module.css";

export interface InboxZeroHeroProps {
  handled: number;
  minutes: number;
  comingBack: number;
  waiting: number;
  /** e.g. "Next check at 10:30. Quiet until 2:00 PM." */
  nextCheck: string;
  photoCredit: string;
  onClose: () => void;
  onComingBack: () => void;
}

/** The reward at inbox zero: a calm photo, what you got done, and a nudge to take a break. */
export function InboxZeroHero({ handled, minutes, comingBack, waiting, nextCheck, photoCredit, onClose, onComingBack }: InboxZeroHeroProps) {
  return (
    <div className={styles.content}>
      <div className={styles.headline}>
        <h1 className={styles.title}>Inbox zero.</h1>
        <p className={styles.sub}>
          You handled {handled} emails in {minutes} minutes. That’s it for now.
        </p>
      </div>
      <div className={styles.stats}>
        <Stat value={handled} label="handled" />
        <Stat value={comingBack} label="coming back later" />
        <Stat value={waiting} label="waiting on you" />
      </div>
      <div className={styles.bottom}>
        <div className={styles.actions}>
          <Button variant="invert" shortcut="⌘W" onClick={onClose}>
            Close and take a break
          </Button>
          <Button variant="glass" onClick={onComingBack}>
            See what’s coming back
          </Button>
        </div>
        <div className={styles.next}>
          <span className={styles.check}>{nextCheck}</span>
          <span className={styles.credit}>{photoCredit}</span>
        </div>
      </div>
    </div>
  );
}
