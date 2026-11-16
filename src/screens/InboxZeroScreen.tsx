import photo from "../assets/inbox-zero/sprout-in-moss.jpg";
import { useShallow } from "zustand/react/shallow";
import { InboxZeroHero, TopBar } from "../design-system/organisms";
import { closeWindow } from "../lib/window";
import { useStore } from "../state/store";
import styles from "./InboxZeroScreen.module.css";
import { SEP } from "../lib/text";

export function InboxZeroScreen() {
  // Only what this screen shows, so a change elsewhere doesn't redraw it.
  const s = useStore(
    useShallow((x) => ({
      threads: x.threads,
      handled: x.handled,
    })),
  );
  const comingBack = s.threads.filter((t) => t.section === "later").length;
  return (
    <div className={styles.screen}>
      <div className={styles.photo} style={{ backgroundImage: `url(${photo})` }} />
      <div className={styles.shade} />
      <TopBar />
      <div className={styles.push} data-tauri-drag-region />
      <InboxZeroHero
        handled={s.handled}
        minutes={14}
        comingBack={comingBack}
        waiting={0}
        nextCheck="Next check at 10:30. Quiet until 2:00 PM."
        photoCredit={`Sprout in the moss${SEP}Runic Earth on Unsplash`}
        onClose={closeWindow}
        onComingBack={() => useStore.setState({ view: "inbox" })}
      />
    </div>
  );
}
